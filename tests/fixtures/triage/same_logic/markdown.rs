//! Fixture: one half of the same function written twice, copied from
//! src/report/markdown.rs. Parsed, never compiled.

fn write_markdown_absolute_heading(
    crappy: usize,
    threshold: f64,
    out: &mut dyn Write,
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
