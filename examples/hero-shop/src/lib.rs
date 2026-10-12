//! The shop crate the README's hero image is rendered from: the triage
//! demo's three duplicate pairs, plus pricing code with partial tests.
//! `docs/assets/hero/render.sh` runs cargo-crap over it, with `pr/` as the
//! pull request.

pub mod shipping;

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

/// Percentage off for a coupon code, if the code is valid.
pub fn coupon_percent(code: &str) -> Option<u8> {
    let code = code.trim();
    if code.is_empty() || code.len() > 16 {
        return None;
    }
    match code.to_ascii_uppercase().as_str() {
        "WELCOME" => Some(10),
        "LOYAL" => Some(15),
        c if c.starts_with("STAFF-") => Some(30),
        _ => None,
    }
}

/// Refund owed for a returned order, in cents.
pub fn refund_cents(total: u64, days_since_delivery: u32, opened: bool, damaged: bool) -> u64 {
    if damaged {
        return total;
    }
    if days_since_delivery > 30 {
        return 0;
    }
    let mut refund = total;
    if opened {
        refund -= refund / 5;
    }
    if days_since_delivery > 14 {
        refund -= refund / 10;
    }
    refund
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parcel(weight_grams: u32, volume_cm3: u32) -> Parcel {
        Parcel { weight_grams, volume_cm3, shipped: true }
    }

    #[test]
    fn a_large_order_gets_ten_percent_off() {
        let lines = [Line { price_cents: 6_000, quantity: 2 }];
        assert_eq!(order_total(&lines), 10_800);
        assert_eq!(order_total(&[Line { price_cents: 500, quantity: 1 }]), 500);
    }

    #[test]
    fn only_shipped_parcels_count() {
        let mut held = parcel(700, 1);
        held.shipped = false;
        assert_eq!(shipped_weight(&[parcel(300, 1), held]), 300);
    }

    #[test]
    fn coupons() {
        assert_eq!(coupon_percent(" welcome "), Some(10));
        assert_eq!(coupon_percent("LOYAL"), Some(15));
        assert_eq!(coupon_percent("staff-anna"), Some(30));
        assert_eq!(coupon_percent(""), None);
        assert_eq!(coupon_percent("NOPE"), None);
        assert_eq!(coupon_percent("A-CODE-FAR-TOO-LONG"), None);
    }

    #[test]
    fn a_damaged_order_is_refunded_in_full() {
        assert_eq!(refund_cents(5_000, 40, true, true), 5_000);
        assert_eq!(refund_cents(5_000, 3, false, false), 5_000);
    }
}
