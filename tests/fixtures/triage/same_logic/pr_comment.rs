//! Fixture: the other half, copied from src/report/pr_comment.rs. Only
//! the parameter order differs. Parsed, never compiled.

fn write_pr_comment_abs_headline(
    out: &mut dyn Write,
    crappy: usize,
    threshold: f64,
) -> Result<()> {
    if crappy == 0 {
        writeln!(out, "## ✅ No CRAP threshold violations")?;
    } else {
        writeln!(
            out,
            "## ⚠️ {crappy} function(s) exceed CRAP threshold {threshold}"
        )?;
    }
    writeln!(out)?;
    Ok(())
}
