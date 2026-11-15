use std::path::Path;
use spur_graph::{GraphFacts, RelationKind};
use spur_graph::extract::{languages::Language, tree_sitter::BytesExtractor};
fn facts(src: &str) -> GraphFacts {
    BytesExtractor::for_language(Language::Rust).unwrap().extract_file_facts(Path::new("src/lib.rs"), src.as_bytes()).unwrap()
}
fn calls(f: &GraphFacts, caller: &str) -> Vec<(String, Option<String>, Option<String>)> {
    let owner=f.nodes.iter().find(|n| n.label==caller).unwrap().node_id;
    f.edges.iter().filter(|e|e.source_node_id==owner && e.relation==RelationKind::Calls).map(|e|(
        e.target_label.clone().unwrap_or_default(),e.scope_text.clone(),e.target_node_id.map(|id| {
            let n=f.nodes.iter().find(|n|n.node_id==id).unwrap();
            format!("{}#{}",n.label,id.get())
        })
    )).collect()
}
#[test]
fn independent_extraction_matrix() {
    let src=r##"
struct Alpha;
struct Beta;
impl Alpha { fn new() -> Self { Self } fn inside() { wrapper!(Self::new()); } fn make<T>() -> Self {Self} }
impl Beta { fn new() -> Self { Self } }
trait Factory { fn build() -> Self; }
struct Genuine;
impl Factory for Genuine { fn build() -> Self {Self} }
fn helper() {}
mod local { pub fn leaf() {} }
fn mixed() { wrapper!(Alpha::new(), foreign::new(), Beta::new(), helper(), foreign::helper()); }
fn nested() { wrapper!(outer!(Alpha::new(), [foreign::new(), {Beta::new()}]), helper()); }
fn paths() { wrapper!(crate::helper(), local::leaf(), foreign::local::leaf(), ::helper(), self::helper(), super::helper()); }
fn trait_forms() { wrapper!(<Genuine as Factory>::build(), <Foreign as Factory>::build(), <Foreign>::new(), <<Foreign as X>::Assoc as Factory>::build()); }
fn generic() { wrapper!(Alpha::make::<Option<Vec<u8>>>(), foreign::make::<fn() -> u8>(), Vec::<Option<Vec<u8>>>::new()); }
fn comment_paths() { wrapper!(foreign /* comment */ :: /* comment */ new(), Alpha /* c */ :: new(), foreign::/*c*/helper()); }
fn noise() { wrapper!("foreign::new() helper()", r#"Alpha::new()"#, b"helper()", /* Alpha::new() */ 1, // foreign::helper()
 2); }
"##;
    let f=facts(src);
    for caller in ["inside","mixed","nested","paths","trait_forms","generic","comment_paths","noise"] {
        let rows=calls(&f,caller);
        println!("{caller}: {rows:?}");
        for (name,scope,target) in &rows {
            if scope.as_deref().is_some_and(|s|s.contains("Foreign")||s.contains("foreign")||s.starts_with("Vec")) {
                assert!(target.is_none(),"foreign target in {caller}: {name} {scope:?} {target:?}");
            }
        }
        if caller=="noise" { assert!(rows.is_empty(),"strings/comments yielded calls"); }
        if ["mixed","nested"].contains(&caller) {
            assert!(rows.iter().any(|(n,s,t)| n=="new" && s.as_deref()==Some("Alpha") && t.is_some()));
            assert!(rows.iter().any(|(n,s,t)| n=="new" && s.as_deref()==Some("Beta") && t.is_some()));
            assert!(rows.iter().any(|(n,s,t)| n=="helper" && s.is_none() && t.is_some()));
        }
        if caller=="inside" { assert!(rows.iter().any(|(n,s,t)| n=="new" && s.as_deref()==Some("Self") && t.is_some())); }
    }
}
