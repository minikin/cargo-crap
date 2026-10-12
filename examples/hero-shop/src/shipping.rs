//! Shipping fees.

use crate::Parcel;

pub enum Zone {
    Domestic,
    Eu,
    World,
}

/// Shipping fee in cents for a parcel going to `zone`.
pub fn shipping_fee(parcel: &Parcel, zone: &Zone, express: bool) -> u64 {
    let mut fee = match zone {
        Zone::Domestic => 490,
        Zone::Eu => 990,
        Zone::World => 1_990,
    };
    if parcel.weight_grams > 2_000 {
        fee += 300;
    }
    if parcel.volume_cm3 > 50_000 {
        fee += 500;
    }
    if express {
        fee *= 2;
    }
    fee
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parcel(weight_grams: u32, volume_cm3: u32) -> Parcel {
        Parcel { weight_grams, volume_cm3, shipped: true }
    }

    #[test]
    fn a_light_domestic_parcel_pays_the_base_fee() {
        assert_eq!(shipping_fee(&parcel(500, 1_000), &Zone::Domestic, false), 490);
    }
}
