#![allow(dead_code)]
enum RawChange { TrustLost(u8) }
struct Pair(u8);
fn case() { let _ = vec![RawChange::TrustLost(1)]; let _ = vec![Pair(2)]; }
fn main() { case(); }
