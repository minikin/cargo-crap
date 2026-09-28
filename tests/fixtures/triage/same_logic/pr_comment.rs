//! Fixture (spec 30, T11): the same logic written twice, copied from
//! src/report/pr_comment.rs. Parsed, never compiled.

fn write_pr_comment_improved_section(
    out: &mut dyn Write,
    b: &DeltaBuckets,
    threshold: f64,
    prefix: &Path,
    links: Option<&SourceLinks>,
    uncovered_hints: bool,
) -> Result<()> {
    if b.improved.is_empty() {
        return Ok(());
    }
    writeln!(out)?;
    writeln!(
        out,
        "<details><summary>↓ {} improved</summary>",
        b.improved.len()
    )?;
    writeln!(out)?;
    write_delta_gfm_header(out, uncovered_hints)?;
    for de in b.improved.iter().take(MAX_ROWS_PER_SECTION) {
        write_pr_comment_row(out, de, threshold, prefix, links, uncovered_hints)?;
    }
    write_truncation_if_capped(out, b.improved.len())?;
    writeln!(out)?;
    writeln!(out, "</details>")?;
    Ok(())
}

fn write_pr_comment_moved_section(
    out: &mut dyn Write,
    b: &DeltaBuckets,
    threshold: f64,
    prefix: &Path,
    links: Option<&SourceLinks>,
    uncovered_hints: bool,
) -> Result<()> {
    if b.moved.is_empty() {
        return Ok(());
    }
    writeln!(out)?;
    writeln!(out, "<details><summary>↔ {} moved</summary>", b.moved.len())?;
    writeln!(out)?;
    write_delta_gfm_header(out, uncovered_hints)?;
    for de in b.moved.iter().take(MAX_ROWS_PER_SECTION) {
        write_pr_comment_row(out, de, threshold, prefix, links, uncovered_hints)?;
    }
    write_truncation_if_capped(out, b.moved.len())?;
    writeln!(out)?;
    writeln!(out, "</details>")?;
    Ok(())
}
