struct Domain;
impl Domain { fn len(&self) -> usize { 999 } }
fn observed(v: Vec<u8>) -> usize { v.len() }
fn expected(d: Domain) -> usize { d.len() }
fn main() { println!("{}:{}", observed(vec![1,2,3]), expected(Domain)); }
