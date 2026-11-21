//! Three copies of one function, named out of alphabetical order, so a
//! listing in source order differs from one in name order.

pub fn zeta_total(items: &[u32], limit: u32) -> u32 {
    let mut total = 0;
    for item in items {
        if *item > limit {
            total += item * 2;
        } else if *item == 0 {
            total += 1;
        } else {
            total += item;
        }
    }
    if total > 100 {
        println!("zeta: {total}");
    }
    total
}

pub fn alpha_total(items: &[u32], limit: u32) -> u32 {
    let mut total = 0;
    for item in items {
        if *item > limit {
            total += item * 2;
        } else if *item == 0 {
            total += 1;
        } else {
            total += item;
        }
    }
    if total > 100 {
        println!("alpha: {total}");
    }
    total
}

pub fn mid_total(items: &[u32], limit: u32) -> u32 {
    let mut total = 0;
    for item in items {
        if *item > limit {
            total += item * 2;
        } else if *item == 0 {
            total += 1;
        } else {
            total += item;
        }
    }
    if total > 100 {
        println!("mid: {total}");
    }
    total
}
