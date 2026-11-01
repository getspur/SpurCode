#![allow(dead_code)]
pub trait Concrete { fn read(&self, value: u8) -> u8; }
impl Concrete for () { fn read(&self, value: u8) -> u8 { value } }
fn expanded_target(value: u8) -> u8 { value }
fn address_only(value: u8) -> u8 { value }
macro_rules! expand_call { ($value:expr) => { expanded_target($value) }; }
#[no_mangle]
pub fn entry(object: &dyn Concrete, x: u8, index: usize) -> u8 {
    let v = /*site:trait_call*/ object.read(x);
    let m = /*site:macro_call*/ expand_call!(v);
    let a = /*site:index_check*/ [7u8, 11u8][index];
    let sum = /*site:overflow_check*/ x + m;
    /*site:explicit_assert*/ assert!(index == 0);
    let _address = /*site:address_cast*/ address_only as fn(u8) -> u8;
    a ^ sum
}
