#![allow(dead_code)]
// Held-out research controls. Each definition is one line to bind exact byte ranges.
pub fn phi_safe(x: u8) -> u8 { let value = if x == 0 { Some(x) } else { None }; if x == 0 { value.unwrap() } else { 0 } }
pub fn phi_bad(x: u8) -> u8 { let value = if x == 0 { Some(x) } else { None }; if x != 0 { value.unwrap() } else { 0 } }
pub fn mutated(x: u8) -> u8 { let mut value = Some(x); if x == 3 { value = None; } if x == 3 { value.unwrap() } else { 0 } }
pub fn shadowed(x: u8) -> u8 { let value = None::<u8>; { let value = Some(x); if x == 7 { return value.unwrap(); } } if x == 8 { value.unwrap() } else { 0 } }
pub fn shortcircuit(x: u8) -> u8 { let value = if x == 0 { Some(x) } else { None }; if x == 0 && value.unwrap() == 0 { 1 } else { 0 } }
pub fn non_dominating(x: u8) -> u8 { let value = if x == 0 { Some(x) } else { None }; if x == 0 { let _ = value.is_some(); } value.unwrap() }
pub fn add_checked(x: u8) -> u8 { x.checked_add(1).unwrap() }
pub fn add_guarded(x: u8) -> u8 { if x < 255 { x.checked_add(1).unwrap() } else { 0 } }
fn helper(x: u8) -> u8 { let value = if x == 0 { Some(x) } else { None }; value.unwrap() }
pub fn call_safe(x: u8) -> u8 { if x == 0 { helper(x) } else { 0 } }
pub fn call_bad(x: u8) -> u8 { if x != 0 { helper(x) } else { 0 } }
pub fn loop_three(x: u8) -> u8 { for step in 0u8..3 { if x == step && step == 2 { return None::<u8>.unwrap(); } } 0 }
pub fn index_bad(x: u8) -> u8 { [10u8, 20, 30][x as usize] }
pub fn index_guarded(x: u8) -> u8 { if x < 3 { [10u8, 20, 30][x as usize] } else { 0 } }
pub fn add_plain(x: u8) -> u8 { x + 1 }
pub struct Lookalike;
impl Lookalike { pub fn unwrap(&self) -> u8 { 7 } }
pub fn lookalike(x: u8) -> u8 { if x == 0 { Lookalike.unwrap() } else { x } }
