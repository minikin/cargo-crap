//! Fixture: two unrelated jobs, an invoice header and an HTTP request head,
//! that only share a run of writeln! calls. Parsed, never compiled.

pub fn write_invoice_header(out: &mut impl Write, invoice: &Invoice) -> fmt::Result {
    writeln!(out, "INVOICE #{}", invoice.number)?;
    writeln!(out, "Issued: {}", invoice.issued)?;
    writeln!(out, "Due: {}", invoice.due)?;
    writeln!(out, "Amount: {}.{:02}", invoice.total_cents / 100, invoice.total_cents % 100)?;
    writeln!(out)?;
    Ok(())
}

pub fn write_request_head(out: &mut impl Write, request: &Request) -> fmt::Result {
    writeln!(out, "{} {} HTTP/1.1", request.method, request.path)?;
    writeln!(out, "Host: {}", request.host)?;
    writeln!(out, "Content-Length: {}", request.body.len())?;
    writeln!(out, "Connection: close")?;
    writeln!(out)?;
    Ok(())
}
