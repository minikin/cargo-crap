use std::io::{self, Write};
use std::path::{Component, Path, PathBuf};

use cargo_crap_core::report::{SourceLinks, display_width};

use crate::changed::verdict::{Change, Counts, Entry, Verdict};

pub(super) const ARROW: &str = "\u{2192}";
pub(super) const DASH: &str = "\u{2014}";

pub const PR_COMMENT_ROWS: usize = 25;

#[derive(Clone, Debug)]
pub(super) struct TextRow {
    verdict: Verdict,
    function: String,
    file: String,
    repo_path: String,
    line: u32,
    base_cyclomatic: Option<u32>,
    cyclomatic: u32,
    coverage: Option<f64>,
    crap: f64,
    change: &'static str,
    reason: String,
}

/// Everything the text forms need, owned, so a row limit can be changed
/// and the section rendered again from the same inputs.
#[derive(Clone, Debug)]
pub(super) struct Text {
    /// Fail, then over, then pass, each in file, line and function order.
    rows: Vec<TextRow>,
    counts: Counts,
    removed: usize,
    skipped: usize,
    rev: String,
    merge_base: String,
    pub(super) fail_changed: bool,
    pub(super) summary: bool,
    links: Option<SourceLinks>,
}

pub(super) struct TextInputs<'a> {
    pub(super) shown: &'a [&'a Entry<'a>],
    pub(super) counts: Counts,
    pub(super) removed: usize,
    pub(super) skipped: usize,
    pub(super) rev: &'a str,
    pub(super) merge_base: &'a str,
    pub(super) fail_changed: bool,
    pub(super) summary: bool,
    pub(super) links: Option<&'a SourceLinks>,
    pub(super) repo_prefix: Option<&'a Path>,
}

fn rank(verdict: Verdict) -> u8 {
    match verdict {
        Verdict::Fail => 0,
        Verdict::Over => 1,
        Verdict::Pass => 2,
    }
}

impl Text {
    pub(super) fn new(inputs: &TextInputs<'_>) -> Self {
        let mut rows: Vec<TextRow> = inputs
            .shown
            .iter()
            .map(|entry| text_row(entry, inputs.repo_prefix))
            .collect();
        rows.sort_by_key(|row| rank(row.verdict));
        Self {
            rows,
            counts: inputs.counts,
            removed: inputs.removed,
            skipped: inputs.skipped,
            rev: inputs.rev.to_owned(),
            merge_base: inputs.merge_base.to_owned(),
            fail_changed: inputs.fail_changed,
            summary: inputs.summary,
            links: inputs.links.cloned(),
        }
    }

    pub(super) fn rows(&self) -> usize {
        self.rows.len()
    }

    fn pass(&self) -> usize {
        self.counts
            .touched
            .saturating_sub(self.counts.fail)
            .saturating_sub(self.counts.over)
    }

    fn counts_line(&self) -> String {
        format!(
            "{} fail, {} over, {} pass among {} touched {}",
            self.counts.fail,
            self.counts.over,
            self.pass(),
            self.counts.touched,
            plural(self.counts.touched, "function", "functions"),
        )
    }

    fn zero_line(&self) -> String {
        format!("No touched functions against {}.", self.rev)
    }

    fn summary_line(&self) -> String {
        format!(
            "Changed functions: {} touched, {} fail.",
            self.counts.touched, self.counts.fail
        )
    }

    fn also_line(&self) -> Option<String> {
        let parts: Vec<String> = [
            (self.counts.allowed, "allowed"),
            (self.counts.missing_skipped, "skipped for missing coverage"),
            (self.counts.coverage_null, "scored without coverage data"),
        ]
        .into_iter()
        .filter(|&(count, _)| count > 0)
        .map(|(count, what)| format!("{count} {what}"))
        .collect();
        (!parts.is_empty()).then(|| format!("Also: {}.", parts.join(", ")))
    }

    fn removed_line(&self) -> Option<String> {
        let mut parts = Vec::new();
        if self.removed > 0 {
            parts.push(format!(
                "Removed: {} {}.",
                self.removed,
                plural(self.removed, "function", "functions")
            ));
        }
        if self.skipped > 0 {
            parts.push(format!(
                "Skipped: {} {}.",
                self.skipped,
                plural(self.skipped, "path", "paths")
            ));
        }
        (!parts.is_empty()).then(|| parts.join(" "))
    }

    fn write_summary(
        &self,
        out: &mut dyn Write,
    ) -> io::Result<()> {
        writeln!(out, "{}", self.summary_line())?;
        self.write_also(out)
    }

    pub(super) fn human(
        &self,
        out: &mut dyn Write,
    ) -> io::Result<()> {
        if self.summary {
            return self.write_summary(out);
        }
        let short = self.merge_base.get(..12).unwrap_or(&self.merge_base);
        writeln!(
            out,
            "Changed functions against {} (merge base {short})",
            self.rev
        )?;
        if self.counts.touched == 0 {
            writeln!(out, "{}", self.zero_line())?;
        } else {
            if !self.rows.is_empty() {
                self.write_table(out)?;
            }
            writeln!(out, "{}.", self.counts_line())?;
        }
        self.write_also(out)?;
        if let Some(removed) = self.removed_line() {
            writeln!(out, "{removed}")?;
        }
        Ok(())
    }

    fn write_table(
        &self,
        out: &mut dyn Write,
    ) -> io::Result<()> {
        let header = [
            "Verdict".to_owned(),
            "Function".to_owned(),
            "Location".to_owned(),
            format!("CC (base {ARROW} head)"),
            "Coverage".to_owned(),
            "CRAP".to_owned(),
            "Change".to_owned(),
        ];
        let rows: Vec<[String; 7]> = self
            .rows
            .iter()
            .map(|row| {
                [
                    verdict_word(row.verdict).to_owned(),
                    row.function.clone(),
                    format!("{}:{}", row.file, row.line),
                    row.cc(),
                    row.coverage
                        .map_or_else(|| DASH.to_owned(), |p| format!("{p:.1}%")),
                    format!("{:.1}", row.crap),
                    row.change.to_owned(),
                ]
            })
            .collect();
        let mut widths = header.each_ref().map(|cell| display_width(cell));
        for row in &rows {
            for (width, cell) in widths.iter_mut().zip(row) {
                *width = (*width).max(display_width(cell));
            }
        }
        let line = |cells: &[String; 7]| {
            let padded: Vec<String> = cells
                .iter()
                .zip(widths)
                .map(|(cell, width)| {
                    let pad = width.saturating_sub(display_width(cell));
                    format!("{cell}{}", " ".repeat(pad))
                })
                .collect();
            padded.join("  ").trim_end().to_owned()
        };
        writeln!(out, "{}", line(&header))?;
        let rule: Vec<String> = widths.iter().map(|&width| "-".repeat(width)).collect();
        writeln!(out, "{}", rule.join("  "))?;
        for row in &rows {
            writeln!(out, "{}", line(row))?;
        }
        Ok(())
    }

    pub(super) fn markdown(
        &self,
        limit: Option<usize>,
        out: &mut dyn Write,
    ) -> io::Result<()> {
        if self.summary {
            return self.write_summary(out);
        }
        let lead = if self.counts.touched == 0 {
            self.zero_line()
        } else {
            format!("{} against {}.", self.counts_line(), self.rev)
        };
        writeln!(out, "## Changed functions\n\n{lead}")?;
        self.write_also(out)?;
        let kept = self.rows.len().min(limit.unwrap_or(usize::MAX));
        self.markdown_table(&self.rows[..kept], out)?;
        let omitted = self.rows.len() - kept;
        if omitted > 0 {
            writeln!(
                out,
                "\nand {omitted} more changed {}",
                plural(omitted, "function", "functions")
            )?;
        }
        if let Some(removed) = self.removed_line() {
            writeln!(out, "\n{removed}")?;
        }
        Ok(())
    }

    fn write_also(
        &self,
        out: &mut dyn Write,
    ) -> io::Result<()> {
        match self.also_line() {
            Some(also) => writeln!(out, "{also}"),
            None => Ok(()),
        }
    }

    fn markdown_table(
        &self,
        rows: &[TextRow],
        out: &mut dyn Write,
    ) -> io::Result<()> {
        if rows.is_empty() {
            return Ok(());
        }
        writeln!(
            out,
            "\n| Verdict | Function | Location | CC (base {ARROW} head) | Coverage | CRAP | Change |"
        )?;
        writeln!(out, "|---|---|---|---|---|---:|---|")?;
        for row in rows {
            writeln!(
                out,
                "| {} | {} | {} | {} | {} | {:.1} | {} |",
                verdict_word(row.verdict),
                cell(&self.linked(code_span(&row.function), row)),
                cell(&self.linked(code_span(&format!("{}:{}", row.file, row.line)), row)),
                row.cc(),
                row.coverage
                    .map_or_else(|| DASH.to_owned(), |p| format!("{p:.1}")),
                row.crap,
                row.change,
            )?;
        }
        Ok(())
    }

    fn linked(
        &self,
        inner: String,
        row: &TextRow,
    ) -> String {
        match &self.links {
            Some(links) => format!(
                "[{inner}]({})",
                links.url_for(Path::new(&row.repo_path), row.line as usize)
            ),
            None => inner,
        }
    }

    pub(super) fn github(
        &self,
        out: &mut dyn Write,
    ) -> io::Result<()> {
        let level = if self.fail_changed {
            "error"
        } else {
            "warning"
        };
        let mut fails: Vec<&TextRow> = self
            .rows
            .iter()
            .filter(|row| row.verdict == Verdict::Fail)
            .collect();
        fails.sort_by(|a, b| {
            (&a.repo_path, a.line, &a.function).cmp(&(&b.repo_path, b.line, &b.function))
        });
        for row in fails {
            writeln!(
                out,
                "::{level} file={},line={},title=Changed function::{}",
                property(&row.repo_path),
                row.line,
                data(&format!("{}: {}", row.function, row.reason)),
            )?;
        }
        Ok(())
    }
}

impl TextRow {
    fn cc(&self) -> String {
        let base = self
            .base_cyclomatic
            .map_or_else(|| DASH.to_owned(), |cc| cc.to_string());
        format!("{base} {ARROW} {}", self.cyclomatic)
    }
}

fn text_row(
    entry: &Entry<'_>,
    repo_prefix: Option<&Path>,
) -> TextRow {
    let head = entry.head;
    let file = head.file.to_string_lossy().into_owned();
    let (change, base_cyclomatic) = match &entry.change {
        Change::New => ("new", None),
        Change::Ambiguous(_) => ("ambiguous", None),
        Change::Modified(base) => ("modified", Some(base.cyclomatic)),
    };
    TextRow {
        verdict: entry.judgement.verdict,
        function: head.name.clone(),
        repo_path: repository_path(repo_prefix, &head.file),
        file,
        line: head.span.start.line,
        base_cyclomatic,
        cyclomatic: head.cyclomatic,
        coverage: entry.coverage,
        crap: entry.crap,
        change,
        reason: entry.judgement.reason.clone(),
    }
}

/// A display path, relative to the invocation, as a path from the
/// repository root: the prefix is where the invocation sits in it.
fn repository_path(
    prefix: Option<&Path>,
    display: &Path,
) -> String {
    let mut parts: Vec<String> = Vec::new();
    let joined: PathBuf = prefix.map_or_else(|| display.to_path_buf(), |p| p.join(display));
    for part in joined.components() {
        match part {
            Component::ParentDir => {
                parts.pop();
            },
            Component::Normal(name) => parts.push(name.to_string_lossy().into_owned()),
            Component::CurDir | Component::RootDir | Component::Prefix(_) => {},
        }
    }
    parts.join("/")
}

/// A code span that holds `text` whatever backticks it contains: the fence
/// is one backtick longer than the longest run inside, padded by a space
/// when the text starts or ends with one.
fn code_span(text: &str) -> String {
    let longest = text.split(|c| c != '`').map(str::len).max().unwrap_or(0);
    let fence = "`".repeat(longest + 1);
    let pad = if text.starts_with('`') || text.ends_with('`') {
        " "
    } else {
        ""
    };
    format!("{fence}{pad}{text}{pad}{fence}")
}

/// GFM ends a table cell at an unescaped pipe, even inside a code span.
fn cell(text: &str) -> String {
    text.replace('|', "\\|")
}

fn verdict_word(verdict: Verdict) -> &'static str {
    match verdict {
        Verdict::Fail => "fail",
        Verdict::Over => "over",
        Verdict::Pass => "pass",
    }
}

fn plural<'w>(
    count: usize,
    one: &'w str,
    many: &'w str,
) -> &'w str {
    if count == 1 { one } else { many }
}

/// The core's workflow-command escaping for message data.
fn data(text: &str) -> String {
    text.replace('%', "%25")
        .replace('\r', "%0D")
        .replace('\n', "%0A")
}

/// The core's workflow-command escaping for a property value.
fn property(text: &str) -> String {
    data(text).replace(':', "%3A").replace(',', "%2C")
}

/// UTF-16 code units: the measure GitHub's comment limit counts in.
#[must_use]
pub fn utf16_len(text: &str) -> usize {
    text.encode_utf16().count()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::changed::verdict::Judgement;
    use crate::{Mode, Row, Side, analyze};

    fn rows() -> Vec<Row> {
        analyze(
            Path::new("src/o.ts"),
            b"function a() {}\nfunction b() {}\nfunction c() {}\nfunction d() {}\n",
            Mode::Strict(Side::Head),
        )
        .unwrap()
        .rows
    }

    fn entry(
        head: &Row,
        verdict: Verdict,
    ) -> Entry<'_> {
        Entry {
            head,
            change: Change::New,
            coverage: None,
            crap: 2.0,
            judgement: Judgement {
                verdict,
                worse: false,
                reason: String::new(),
            },
        }
    }

    fn inputs<'a>(
        shown: &'a [&'a Entry<'a>],
        counts: Counts,
    ) -> TextInputs<'a> {
        TextInputs {
            shown,
            counts,
            removed: 0,
            skipped: 0,
            rev: "main",
            merge_base: "0123456789abcdef",
            fail_changed: false,
            summary: false,
            links: None,
            repo_prefix: None,
        }
    }

    fn text(
        shown: &[&Entry<'_>],
        counts: Counts,
        removed: usize,
        skipped: usize,
    ) -> Text {
        Text::new(&TextInputs {
            shown,
            counts,
            removed,
            skipped,
            rev: "main",
            merge_base: "0123456789abcdef",
            fail_changed: false,
            summary: false,
            links: None,
            repo_prefix: None,
        })
    }

    #[test]
    fn rows_are_ordered_fail_then_over_then_pass_keeping_file_order_within_each() {
        let rows = rows();
        let entries = [
            entry(&rows[0], Verdict::Pass),
            entry(&rows[1], Verdict::Fail),
            entry(&rows[2], Verdict::Over),
            entry(&rows[3], Verdict::Fail),
        ];
        let shown: Vec<&Entry<'_>> = entries.iter().collect();
        let text = text(&shown, Counts::default(), 0, 0);
        let names: Vec<&str> = text.rows.iter().map(|row| row.function.as_str()).collect();
        assert_eq!(names, ["b", "d", "c", "a"]);
    }

    #[test]
    fn counts_of_one_take_the_singular() {
        let one = Counts {
            touched: 1,
            fail: 1,
            ..Counts::default()
        };
        let text1 = text(&[], one, 1, 1);
        assert_eq!(
            text1.counts_line(),
            "1 fail, 0 over, 0 pass among 1 touched function"
        );
        assert_eq!(
            text1.removed_line().as_deref(),
            Some("Removed: 1 function. Skipped: 1 path.")
        );
        let two = Counts {
            touched: 2,
            over: 1,
            ..Counts::default()
        };
        let text2 = text(&[], two, 2, 0);
        assert_eq!(
            text2.counts_line(),
            "0 fail, 1 over, 1 pass among 2 touched functions"
        );
        assert_eq!(
            text2.removed_line().as_deref(),
            Some("Removed: 2 functions.")
        );
        assert_eq!(
            text(&[], Counts::default(), 0, 3).removed_line().as_deref(),
            Some("Skipped: 3 paths.")
        );
        assert_eq!(text(&[], Counts::default(), 0, 0).removed_line(), None);
        assert_eq!(plural(0, "path", "paths"), "paths");
    }

    #[test]
    fn the_also_line_lists_positive_counts_in_a_fixed_order() {
        let counts = |allowed, missing_skipped, coverage_null| Counts {
            allowed,
            missing_skipped,
            coverage_null,
            ..Counts::default()
        };
        assert_eq!(text(&[], counts(0, 0, 0), 0, 0).also_line(), None);
        assert_eq!(
            text(&[], counts(1, 0, 2), 0, 0).also_line().as_deref(),
            Some("Also: 1 allowed, 2 scored without coverage data.")
        );
        assert_eq!(
            text(&[], counts(0, 3, 0), 0, 0).also_line().as_deref(),
            Some("Also: 3 skipped for missing coverage.")
        );
    }

    #[test]
    fn length_is_counted_in_utf16_code_units() {
        assert_eq!(utf16_len(""), 0);
        assert_eq!(utf16_len("a\u{e9}\u{1f600}"), 4);
        assert_eq!("a\u{e9}\u{1f600}".len(), 7);
    }

    #[test]
    fn a_display_path_is_projected_onto_the_repository() {
        let prefix = Path::new("packages/app");
        assert_eq!(
            repository_path(Some(prefix), Path::new("../shared/x.ts")),
            "packages/shared/x.ts"
        );
        assert_eq!(
            repository_path(Some(prefix), Path::new("./src/x.ts")),
            "packages/app/src/x.ts"
        );
        assert_eq!(repository_path(None, Path::new("src/x.ts")), "src/x.ts");
    }

    #[test]
    fn markdown_cells_survive_pipes_and_backticks() {
        assert_eq!(code_span("src/a.ts:1"), "`src/a.ts:1`");
        assert_eq!(code_span("a`b"), "``a`b``");
        assert_eq!(code_span("`x"), "`` `x ``");
        assert_eq!(code_span("x``"), "``` x`` ```");
        assert_eq!(cell("`src/a|b.ts:1`"), "`src/a\\|b.ts:1`");
        assert_eq!(
            cell("[`a|b`](https://h.invalid/a|b#L1)"),
            "[`a\\|b`](https://h.invalid/a\\|b#L1)"
        );
    }

    #[test]
    fn markdown_rows_escape_linked_and_unlinked_locations() {
        let rows = analyze(
            Path::new("src/a|b.ts"),
            b"function f() {}\n",
            Mode::Strict(Side::Head),
        )
        .unwrap()
        .rows;
        let entries = [entry(&rows[0], Verdict::Pass)];
        let shown: Vec<&Entry<'_>> = entries.iter().collect();
        let counts = Counts {
            touched: 1,
            ..Counts::default()
        };
        let links = SourceLinks::new("https://h.invalid/o/r".to_owned(), "abc".to_owned());
        for linked in [None, Some(&links)] {
            let text = Text::new(&TextInputs {
                links: linked,
                ..inputs(&shown, counts)
            });
            let mut out = Vec::new();
            text.markdown(None, &mut out).unwrap();
            let body = String::from_utf8(out).unwrap();
            let row = body
                .lines()
                .find(|line| line.starts_with("| pass"))
                .unwrap();
            assert_eq!(row.matches(" | ").count(), 6, "{row}");
            assert!(row.contains("`src/a\\|b.ts:1`"), "{row}");
        }
    }

    #[test]
    fn the_human_table_pads_by_display_width() {
        let rows = analyze(
            Path::new("src/x.ts"),
            "function f() {}\nfunction \u{540d}() {}\nfunction e\u{301}() {}\n".as_bytes(),
            Mode::Strict(Side::Head),
        )
        .unwrap()
        .rows;
        let entries: Vec<Entry<'_>> = rows.iter().map(|row| entry(row, Verdict::Pass)).collect();
        let shown: Vec<&Entry<'_>> = entries.iter().collect();
        let counts = Counts {
            touched: 3,
            ..Counts::default()
        };
        let text = Text::new(&inputs(&shown, counts));
        let mut out = Vec::new();
        text.human(&mut out).unwrap();
        let body = String::from_utf8(out).unwrap();
        let lines: Vec<&str> = body.lines().collect();
        let column = |line: &str, needle: &str| {
            let at = line
                .find(needle)
                .unwrap_or_else(|| panic!("{needle} in {line}"));
            display_width(&line[..at])
        };
        let location = column(lines[1], "Location");
        for row in &lines[3..6] {
            assert_eq!(column(row, "src/"), location, "{body}");
        }
        assert_eq!(
            column(lines[3], "\u{2192}"),
            column(lines[4], "\u{2192}"),
            "{body}"
        );
        assert_eq!(
            column(lines[3], "\u{2192}"),
            column(lines[5], "\u{2192}"),
            "{body}"
        );
    }

    #[test]
    fn workflow_commands_escape_data_and_properties_as_the_core_does() {
        assert_eq!(data("50%\r\n:,"), "50%25%0D%0A:,");
        assert_eq!(property("a:b,c%"), "a%3Ab%2Cc%25");
    }
}
