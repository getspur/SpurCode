#![allow(dead_code)]
pub struct Alpha;
pub struct Beta;
pub struct Pretender;
impl Alpha { pub fn mark(&self) -> usize { 11 } }
impl Beta { pub fn mark(&self) -> usize { 22 } }
impl Pretender { pub fn len(&self) -> usize { 77 } }
pub fn receivers() -> [usize; 9] {
    let a = Alpha;
    let one = /*site:alpha_inferred*/ a.mark();
    let b = Beta;
    let two = /*site:beta_inferred*/ b.mark();
    let typed: Alpha = Alpha;
    let three = /*site:alpha_typed*/ typed.mark();
    let borrowed: &Beta = &b;
    let four = /*site:beta_ref*/ borrowed.mark();
    let value = Alpha;
    let five = /*site:before_rebind*/ value.mark();
    let value = Beta;
    let six = /*site:after_rebind*/ value.mark();
    let text: &str = "abc";
    let seven = /*site:std_len*/ text.len();
    let custom = Pretender;
    let eight = /*site:custom_len*/ custom.len();
    let nine = /*site:literal_len*/ "abcd".len();
    [one, two, three, four, five, six, seven, eight, nine]
}
