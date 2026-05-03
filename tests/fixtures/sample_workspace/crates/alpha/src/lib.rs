pub fn alpha_clean(x: u32) -> u32 {
    x + 1
}

pub fn alpha_branchy(x: u32) -> u32 {
    if x > 10 {
        x * 2
    } else if x > 5 {
        x + 5
    } else if x > 0 {
        x + 1
    } else {
        0
    }
}
