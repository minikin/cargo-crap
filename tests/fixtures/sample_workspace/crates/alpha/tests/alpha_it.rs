//! Integration-test fixture: excluded by default (spec 14), analyzed only
//! with `--no-default-excludes`.

pub fn alpha_integration_helper(x: u32) -> u32 {
    if x > 3 { x } else { 0 }
}

#[test]
fn alpha_clean_adds_one() {
    assert_eq!(alpha_integration_helper(4), 4);
}
