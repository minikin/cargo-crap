//! A small shop backend with three pairs of look-alike functions. Each pair
//! scores high on structural similarity, and each is a different kind of
//! duplication.

use std::fmt::{self, Write};

pub struct Line {
    pub price_cents: u64,
    pub quantity: u32,
}

pub struct Parcel {
    pub weight_grams: u32,
    pub volume_cm3: u32,
    pub shipped: bool,
}

/// Total of an order, with 10% off above 100.00.
pub fn order_total(lines: &[Line]) -> u64 {
    let mut total = 0;
    for line in lines {
        total += line.price_cents * u64::from(line.quantity);
    }
    if total > 10_000 {
        total -= total / 10;
    }
    total
}

/// Total of a quote, with 10% off above 100.00.
pub fn quote_total(items: &[Line]) -> u64 {
    let mut sum = 0;
    for item in items {
        sum += item.price_cents * u64::from(item.quantity);
    }
    if sum > 10_000 {
        sum -= sum / 10;
    }
    sum
}

/// Weight of the parcels that have left the warehouse.
pub fn shipped_weight(parcels: &[Parcel]) -> u32 {
    let mut grams = 0;
    for parcel in parcels {
        if parcel.shipped {
            grams += parcel.weight_grams;
        }
    }
    grams
}

/// Volume of the parcels that have left the warehouse.
pub fn shipped_volume(parcels: &[Parcel]) -> u32 {
    let mut volume = 0;
    for parcel in parcels {
        if parcel.shipped {
            volume += parcel.volume_cm3;
        }
    }
    volume
}

/// The receipt printed at the till.
pub fn write_receipt(
    out: &mut impl Write,
    customer: &str,
    total: u64,
) -> fmt::Result {
    writeln!(out, "RECEIPT")?;
    writeln!(out, "Customer: {customer}")?;
    writeln!(out, "Total: {}.{:02}", total / 100, total % 100)?;
    writeln!(out, "Paid by card")?;
    writeln!(out, "Thank you for shopping with us")?;
    Ok(())
}

/// The label stuck on a parcel.
pub fn write_shipping_label(
    out: &mut impl Write,
    name: &str,
    city: &str,
) -> fmt::Result {
    writeln!(out, "SHIP TO")?;
    writeln!(out, "{name}")?;
    writeln!(out, "{city}")?;
    writeln!(out, "Handle with care")?;
    writeln!(out, "Keep dry")?;
    Ok(())
}
