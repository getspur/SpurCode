#![allow(dead_code)]
pub fn plain(n: u8) -> u16 {
    if n == 0 { 1 } else { 1 + /*site:plain_self*/ plain(n - 1) }
}
pub fn shadowed(n: u8) -> u16 {
    let before = if n == 0 { 0 } else { /*site:before_shadow*/ shadowed(n - 1) };
    let middle = {
        let shadowed = |v: u8| -> u16 { 100 + v as u16 };
        /*site:closure_shadow*/ shadowed(n)
    };
    let after = if n == 0 { 0 } else { /*site:after_shadow*/ shadowed(n - 1) };
    before + middle + after
}
pub fn nested(n: u8) -> u16 {
    fn nested(local: u8) -> u16 { 200 + local as u16 }
    /*site:local_homonym*/ nested(n)
}
pub mod other {
    pub fn same(value: u8) -> u16 { 300 + value as u16 }
}
pub fn same(n: u8) -> u16 {
    /*site:module_homonym*/ other::same(n)
}
pub fn qualified(n: u8) -> u16 {
    if n == 0 { 1 } else { 1 + /*site:qualified_self*/ self::qualified(n - 1) }
}
