#![allow(dead_code)]
mod outer { pub mod local { pub fn deep() -> u8 { 7 } } }
fn max() -> u8 { 42 }
fn plain() { let _=vec![outer::local::deep()]; }
fn spaces() { let _=vec![outer :: local :: deep()]; }
fn internal_block() { let _=vec![outer /* qualifier */ :: local::deep()]; }
fn internal_line() { let _=vec![outer // qualifier
 :: local::deep()]; }
fn trailing_block() { let _=vec![outer::local /* qualifier */ ::deep()]; }
fn trailing_line() { let _=vec![outer::local // qualifier
 ::deep()]; }
fn all_comments() { let _=vec![outer /* a */ :: /* b */ local /* c */ :: /* d */ deep /* e */ ()]; }
fn foreign_standard() { let _=vec![std /* foreign */ :: cmp :: max(1u8,2u8)]; }
fn main() { plain(); spaces(); internal_block(); internal_line(); trailing_block(); trailing_line(); all_comments(); foreign_standard(); println!("path-probes-ok"); }
