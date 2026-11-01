#![allow(dead_code)]
pub struct Probe;
pub static METHOD_HITS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
pub static FREE_HITS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
impl Probe { pub fn mark(&self) -> usize {
    METHOD_HITS.fetch_add(1, std::sync::atomic::Ordering::Relaxed); 31
} }
pub fn ping() -> usize {
    FREE_HITS.fetch_add(1, std::sync::atomic::Ordering::Relaxed); 9
}
pub fn bodies() -> [usize; 4] {
    let p = Probe;
    let a = /*site:repeat_one*/ p.mark();
    let b = /*site:repeat_two*/ p.mark();
    let _unexecuted = /*body:closure:start*/ || {
        /*site:closure_method*/ p.mark() + /*site:closure_free*/ ping()
    } /*body:closure:end*/;
    let _future = /*body:async:start*/ async {
        /*site:async_method*/ p.mark() + /*site:async_free*/ ping()
    } /*body:async:end*/;
    /*body:nested:start*/ fn never_called() -> usize {
        let p = Probe;
        /*site:nested_method*/ p.mark() + /*site:nested_free*/ ping()
    } /*body:nested:end*/
    let c = /*site:free_one*/ ping();
    let d = /*site:free_two*/ ping();
    [a, b, c, d]
}
