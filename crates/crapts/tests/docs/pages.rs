use crate::markers::{self, Marker};
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

pub const ROOT_MARKER: &str = "<!-- output:";
pub const README_SECTION: &str = "crapts for TypeScript";

pub fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

pub fn read(relative: &str) -> String {
    fs::read_to_string(repo_root().join(relative))
        .unwrap_or_else(|e| panic!("reading {relative}: {e}"))
        .replace("\r\n", "\n")
}

/// Markdown text and where it came from: `path` is relative to the
/// repository root and `first_line` is the file line of `text`'s first line.
#[derive(Clone)]
pub struct Page {
    pub path: String,
    pub first_line: usize,
    pub text: String,
}

impl Page {
    pub fn new(
        path: &str,
        text: &str,
    ) -> Page {
        Page {
            path: path.to_owned(),
            first_line: 1,
            text: text.replace("\r\n", "\n"),
        }
    }

    pub fn load(path: &str) -> Page {
        Page::new(path, &read(path))
    }

    pub fn dir(&self) -> PathBuf {
        let path = Path::new(&self.path);
        path.parent().map(Path::to_path_buf).unwrap_or_default()
    }

    pub fn with(
        &self,
        from: &str,
        to: &str,
    ) -> Page {
        assert!(
            self.text.contains(from),
            "{}: no `{from}` to edit",
            self.path
        );
        Page {
            text: self.text.replacen(from, to, 1),
            ..self.clone()
        }
    }

    pub fn lines(&self) -> Vec<Line<'_>> {
        classify(&self.text, self.first_line)
    }

    pub fn blocks(&self) -> Vec<Block> {
        blocks(&self.lines())
    }

    pub fn at(
        &self,
        line: usize,
    ) -> String {
        format!("{}:{line}", self.path)
    }
}

pub fn crapts_pages() -> Vec<Page> {
    let mut paths = Vec::new();
    let mut dirs = vec![repo_root().join("docs/crapts")];
    while let Some(dir) = dirs.pop() {
        for entry in fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                dirs.push(path);
            } else if path.extension().is_some_and(|e| e == "md") {
                paths.push(path);
            }
        }
    }
    let root = repo_root();
    let mut pages: Vec<Page> = paths
        .iter()
        .map(|path| {
            let relative = path.strip_prefix(&root).unwrap();
            Page::load(&relative.to_string_lossy().replace('\\', "/"))
        })
        .collect();
    pages.sort_by(|a, b| a.path.cmp(&b.path));
    pages
}

pub fn sections(
    page: &Page,
    title: &str,
) -> Vec<Page> {
    let lines = page.lines();
    let mut found = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        let Some((level, heading)) = line.heading() else {
            continue;
        };
        if heading != title {
            continue;
        }
        let end = lines[i + 1..]
            .iter()
            .position(|l| l.heading().is_some_and(|(other, _)| other <= level))
            .map_or(lines.len(), |offset| i + 1 + offset);
        let text: Vec<&str> = lines[i..end].iter().map(|l| l.text).collect();
        found.push(Page {
            path: page.path.clone(),
            first_line: line.number,
            text: text.join("\n") + "\n",
        });
    }
    found
}

pub fn section(
    page: &Page,
    title: &str,
) -> Result<Page, String> {
    let mut found = sections(page, title);
    match found.len() {
        1 => Ok(found.remove(0)),
        n => Err(format!(
            "{}: {n} sections headed \"{title}\", expected one",
            page.path
        )),
    }
}

pub fn readme_sections(readme: &Page) -> Vec<Page> {
    sections(readme, README_SECTION)
}

pub fn table_rows(page: &Page) -> Vec<(usize, Vec<String>)> {
    let mut rows = Vec::new();
    let mut in_table = false;
    for line in page.lines() {
        let row = !line.fenced && !line.fence && line.text.trim_start().starts_with('|');
        if !row {
            in_table = false;
            continue;
        }
        let cells: Vec<String> = line
            .text
            .trim()
            .trim_matches('|')
            .split('|')
            .map(|c| c.trim().to_owned())
            .collect();
        let separator = cells
            .iter()
            .all(|c| !c.is_empty() && c.chars().all(|ch| matches!(ch, '-' | ':')));
        if in_table && !separator {
            rows.push((line.number, cells));
        }
        in_table = true;
    }
    rows
}

pub fn unquote(cell: &str) -> &str {
    cell.trim().trim_matches('`')
}

pub fn checked_pages() -> Vec<Page> {
    let mut pages = crapts_pages();
    pages.extend(readme_sections(&Page::load("README.md")));
    pages
}

pub struct Line<'a> {
    pub number: usize,
    pub text: &'a str,
    pub fenced: bool,
    pub fence: bool,
}

impl Line<'_> {
    pub fn heading(&self) -> Option<(usize, &str)> {
        if self.fenced || self.fence {
            return None;
        }
        let level = self.text.chars().take_while(|c| *c == '#').count();
        let title = self.text[level..].strip_prefix(' ')?;
        (1..=6).contains(&level).then_some((level, title.trim()))
    }
}

fn fence_of(line: &str) -> Option<String> {
    let trimmed = line.trim_start();
    let ch = trimmed.chars().next().filter(|c| *c == '`' || *c == '~')?;
    let run: String = trimmed.chars().take_while(|c| *c == ch).collect();
    (run.len() >= 3).then_some(run)
}

fn classify(
    text: &str,
    first_line: usize,
) -> Vec<Line<'_>> {
    let mut open: Option<String> = None;
    let mut lines = Vec::new();
    for (i, raw) in text.lines().enumerate() {
        let fence = fence_of(raw);
        let closes = matches!((&open, &fence), (Some(o), Some(f)) if f.starts_with(o.as_str())
            && raw.trim().len() == f.len());
        let is_fence = closes || (open.is_none() && fence.is_some());
        lines.push(Line {
            number: first_line + i,
            text: raw,
            fenced: open.is_some() && !closes,
            fence: is_fence,
        });
        if closes {
            open = None;
        } else if open.is_none() {
            open = fence;
        }
    }
    lines
}

pub struct Block {
    pub open_line: usize,
    pub info: String,
    pub body: String,
}

fn blocks(lines: &[Line]) -> Vec<Block> {
    let mut found: Vec<Block> = Vec::new();
    let mut current: Option<(Block, Vec<&str>)> = None;
    for line in lines {
        match (&mut current, line.fence) {
            (None, true) => {
                let info = line.text.trim().trim_start_matches(['`', '~']).trim();
                let block = Block {
                    open_line: line.number,
                    info: info.to_owned(),
                    body: String::new(),
                };
                current = Some((block, Vec::new()));
            },
            (Some(_), true) => {
                let (mut block, body) = current.take().unwrap();
                block.body = body.iter().flat_map(|l| [*l, "\n"]).collect();
                found.push(block);
            },
            (Some((_, body)), false) => body.push(line.text),
            (None, false) => {},
        }
    }
    found
}

#[derive(Clone)]
pub struct Sample {
    pub page: String,
    pub line: usize,
    pub marker: Marker,
    pub block: String,
}

impl Sample {
    pub fn label(&self) -> String {
        format!("{}:{} `{}`", self.page, self.line, self.marker)
    }
}

/// The root docs harness tests raw lines, fenced or not, so this does too.
pub fn root_marker_lines(page: &Page) -> Result<(), String> {
    for line in page.lines() {
        if line.text.starts_with(ROOT_MARKER) {
            return Err(format!(
                "{}: `{}` is a cargo-crap marker, and the root docs check would run cargo-crap on it",
                page.at(line.number),
                line.text
            ));
        }
    }
    Ok(())
}

pub fn samples(page: &Page) -> Result<Vec<Sample>, String> {
    root_marker_lines(page)?;
    let lines = page.lines();
    let blocks = page.blocks();
    let mut found = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        if line.fenced || line.fence || !line.text.trim().starts_with(markers::PREFIX) {
            continue;
        }
        let marker = markers::parse(line.text.trim())
            .map_err(|reason| format!("{}: {reason}", page.at(line.number)))?;
        let next = lines[i + 1..].iter().find(|l| !l.text.trim().is_empty());
        let block = next
            .filter(|l| l.fence)
            .and_then(|l| blocks.iter().find(|b| b.open_line == l.number))
            .ok_or_else(|| {
                format!(
                    "{}: marker `{}` is not followed by its fenced block",
                    page.at(line.number),
                    line.text.trim()
                )
            })?;
        found.push(Sample {
            page: page.path.clone(),
            line: line.number,
            marker,
            block: block.body.clone(),
        });
    }
    Ok(found)
}

pub fn literal_versions(
    page: &Page,
    version: &str,
) -> Result<(), String> {
    let literal = format!("\"{version}\"");
    for line in page.lines() {
        let found = line
            .text
            .split("\"version\":")
            .skip(1)
            .any(|after| after.trim_start().starts_with(&literal));
        if found {
            return Err(format!(
                "{}: the literal version {version} belongs as <version>",
                page.at(line.number)
            ));
        }
    }
    Ok(())
}

pub fn first_difference(
    shown: &str,
    actual: &str,
) -> Option<String> {
    if shown == actual {
        return None;
    }
    let shown_lines: Vec<&str> = shown.split_inclusive('\n').collect();
    let actual_lines: Vec<&str> = actual.split_inclusive('\n').collect();
    let at = (0..shown_lines.len().max(actual_lines.len()))
        .find(|&i| shown_lines.get(i) != actual_lines.get(i))
        .unwrap_or(0);
    Some(format!(
        "line {} of the block differs:\n  shown:  {:?}\n  actual: {:?}",
        at + 1,
        shown_lines.get(at).copied().unwrap_or("<end of block>"),
        actual_lines.get(at).copied().unwrap_or("<end of output>")
    ))
}

pub fn command_lines(page: &Page) -> Vec<(usize, String)> {
    let mut found = Vec::new();
    for block in page.blocks() {
        let lang = block.info.split_whitespace().next().unwrap_or_default();
        if !["bash", "sh", "console", "shell"].contains(&lang) {
            continue;
        }
        let mut pending: Option<(usize, String)> = None;
        for (offset, raw) in block.body.lines().enumerate() {
            let number = block.open_line + 1 + offset;
            let (start, text) = match pending.take() {
                Some((start, text)) => (start, format!("{text} {}", raw.trim())),
                None => (number, raw.strip_prefix("$ ").unwrap_or(raw).to_owned()),
            };
            if let Some(joined) = text.strip_suffix('\\') {
                pending = Some((start, joined.trim_end().to_owned()));
            } else {
                found.push((start, text));
            }
        }
        found.extend(pending);
    }
    found
}

pub fn crapts_arguments(line: &str) -> Result<Option<Vec<String>>, String> {
    let is_crapts =
        line == "crapts" || line.starts_with("crapts ") || line.starts_with("npx @minikin/crapts");
    if !is_crapts {
        return Ok(None);
    }
    let words = markers::split(line)?;
    let skip = match words.as_slice() {
        [first, ..] if first == "crapts" => 1,
        [npx, package, ..]
            if npx == "npx"
                && (package == "@minikin/crapts" || package.starts_with("@minikin/crapts@")) =>
        {
            2
        },
        _ => return Ok(None),
    };
    Ok(Some(words[skip..].to_vec()))
}

pub fn command_runs(args: &[String]) -> Vec<Vec<String>> {
    let mut probe = vec!["--print-config".to_owned()];
    probe.extend(
        args.iter()
            .filter(|a| !markers::INFORMATIONAL.contains(&a.as_str()))
            .cloned(),
    );
    let informational = args
        .iter()
        .any(|a| markers::INFORMATIONAL.contains(&a.as_str()));
    if informational && args != probe.as_slice() {
        vec![probe, args.to_vec()]
    } else {
        vec![probe]
    }
}

struct Links {
    inline: Vec<(usize, String)>,
    defined: Vec<(usize, String, String)>,
    prose: String,
}

fn prose_of(text: &str) -> String {
    text.split('`').step_by(2).collect::<Vec<_>>().join(" ")
}

fn scan_links(page: &Page) -> Links {
    let mut found = Links {
        inline: Vec::new(),
        defined: Vec::new(),
        prose: String::new(),
    };
    let lines = page.lines();
    for (i, line) in lines.iter().enumerate() {
        if line.fenced || line.fence {
            continue;
        }
        if let Some((label, after)) = reference_definition(line.text) {
            let next = lines[i + 1..].iter().find(|l| !l.text.trim().is_empty());
            let (number, text) = match (after.trim(), next) {
                ("", Some(next)) => (next.number, next.text.trim()),
                (text, _) => (line.number, text),
            };
            let (target, _) = destination(text);
            if local(target) {
                found
                    .defined
                    .push((number, label.to_lowercase(), target.to_owned()));
            }
            continue;
        }
        let prose = prose_of(line.text);
        let mut rest = prose.as_str();
        while let Some(at) = rest.find("](") {
            rest = &rest[at + 2..];
            let (target, consumed) = destination(rest);
            if local(target) {
                found.inline.push((line.number, target.to_owned()));
            }
            rest = &rest[consumed..];
        }
        found.prose.push_str(&prose.to_lowercase());
        found.prose.push('\n');
    }
    found
}

/// Every relative link target, unused reference definitions included.
pub fn links(page: &Page) -> Vec<(usize, String)> {
    let found = scan_links(page);
    let mut all = found.inline;
    all.extend(
        found
            .defined
            .into_iter()
            .map(|(line, _, target)| (line, target)),
    );
    all
}

/// The targets a reader can follow: inline links and the reference
/// definitions some link uses.
pub fn clickable_links(page: &Page) -> Vec<(usize, String)> {
    let found = scan_links(page);
    let mut all = found.inline;
    for (line, label, target) in found.defined {
        let shortcut = format!("[{label}]");
        let used = found
            .prose
            .match_indices(&shortcut)
            .any(|(at, _)| !found.prose[at + shortcut.len()..].starts_with(['(', ':']));
        if used {
            all.push((line, target));
        }
    }
    all
}

fn local(target: &str) -> bool {
    !target.is_empty() && !target.contains("://") && !target.starts_with("mailto:")
}

/// The destination and the bytes of `text` it spans.
fn destination(text: &str) -> (&str, usize) {
    if let Some(inner) = text.strip_prefix('<') {
        let target = inner.split('>').next().unwrap_or_default();
        return (target, (1 + target.len() + 1).min(text.len()));
    }
    let end = text
        .find(|c: char| c.is_whitespace() || c == ')')
        .unwrap_or(text.len());
    (&text[..end], end)
}

fn reference_definition(line: &str) -> Option<(&str, &str)> {
    let rest = line.trim_start().strip_prefix('[')?;
    let (label, after) = rest.split_once("]:")?;
    (!label.is_empty() && !label.contains(']')).then_some((label, after))
}

/// GitHub's anchor for a heading.
pub fn slug(heading: &str) -> String {
    heading
        .to_lowercase()
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == ' ' || *c == '-' || *c == '_')
        .map(|c| if c == ' ' { '-' } else { c })
        .collect()
}

pub fn anchors(text: &str) -> BTreeSet<String> {
    let mut seen = BTreeSet::new();
    for line in classify(text, 1) {
        if let Some((_, title)) = line.heading() {
            let base = slug(title);
            let mut anchor = base.clone();
            let mut n = 0;
            while seen.contains(&anchor) {
                n += 1;
                anchor = format!("{base}-{n}");
            }
            seen.insert(anchor);
        }
    }
    seen
}

pub fn check_link(
    page: &Page,
    line: usize,
    target: &str,
) -> Result<(), String> {
    let (file, anchor) = match target.split_once('#') {
        Some((file, anchor)) => (file, Some(anchor)),
        None => (target, None),
    };
    let text = if file.is_empty() {
        page.text.clone()
    } else {
        let path = repo_root().join(page.dir()).join(file);
        fs::read_to_string(&path)
            .map_err(|_| format!("{}: the link target {target} does not exist", page.at(line)))?
    };
    match anchor {
        Some(anchor) if !anchors(&text).contains(anchor) => Err(format!(
            "{}: the link target {target} names no heading #{anchor}",
            page.at(line)
        )),
        _ => Ok(()),
    }
}

pub fn box_rows(block: &str) -> Vec<Vec<String>> {
    block
        .lines()
        .filter(|l| l.starts_with('\u{2502}'))
        .map(|l| {
            l.trim_matches('\u{2502}')
                .split('\u{2506}')
                .map(|c| c.trim().to_owned())
                .collect()
        })
        .collect()
}

pub fn inline_code(text: &str) -> Vec<&str> {
    text.split('`').skip(1).step_by(2).collect()
}

pub fn prose_code(page: &Page) -> Vec<(usize, String)> {
    page.lines()
        .iter()
        .filter(|l| !l.fenced && !l.fence)
        .flat_map(|l| {
            inline_code(l.text)
                .into_iter()
                .map(|c| (l.number, c.to_owned()))
        })
        .collect()
}
