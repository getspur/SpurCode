#![allow(dead_code)]
struct LocalFactory;
impl LocalFactory {
    fn new() -> Self { Self }
}
struct LocalDropper;
impl Drop for LocalDropper {
    fn drop(&mut self) {}
}
fn observed_macro() {
    let values: Vec<u8> = Vec::default();
    assert_eq!(values, Vec::new());
}
fn observed_drop() {
    let lock = std::sync::Mutex::new(1_i32);
    let guard = lock.lock().unwrap();
    drop(guard);
}
fn helper() -> usize { 7 }
fn observed_helper() -> usize { helper() }
fn main() {
    observed_macro();
    observed_drop();
    assert_eq!(observed_helper(), 7);
    println!("binding-control-ok");
}
