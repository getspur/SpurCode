#![allow(dead_code)]
struct Factory;
impl Factory {
    fn new() -> Self { Self }
    fn via_self() -> Self {
        let mut items = vec![Self::new()];
        items.pop().unwrap()
    }
}
fn plain() { let _ = vec![Factory::new()]; }
fn spaced() { let _ = vec![Factory :: new()]; }
fn block_before_separator() { let _ = vec![Factory /* qualifier */ :: new()]; }
fn block_after_separator() { let _ = vec![Factory:: /* callee */ new()]; }
fn line_before_separator() {
    let _ = vec![Factory // qualifier
        :: new()];
}
fn main() {
    plain(); spaced(); block_before_separator(); block_after_separator();
    line_before_separator(); let _ = Factory::via_self();
    println!("scope-probe-ok");
}
