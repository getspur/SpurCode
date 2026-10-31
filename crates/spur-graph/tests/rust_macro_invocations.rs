use std::collections::{BTreeMap, BTreeSet};

use serde::Deserialize;
use spur_graph::{
    build_facts, git_blob_oid, macro_invocation_site_id, GraphFacts, MacroEvidenceSource,
    MacroExecutionPhase, MacroInvocationEvidenceSet, MacroResolutionEvidence, MacroScopeEvidence,
    MacroScopeKind, MacroScopeRef, MacroSinkFamily, MacroTestContext, NodeKind, RelationKind,
    MACRO_INVOCATION_SCHEMA_VERSION,
};

// Every sNN annotation identifies one direct AST site. The expected spans and
// scopes below were enumerated from these exact bytes before implementation.
const SOURCE: &str = r###"#![allow(dead_code)]
    /*s01*/ register!();
type Alias =     /*s02*/ type_macro!();
const C: () =     /*s03*/ panic!("const");
static S: () =     /*s04*/ todo!("static");
mod plain {
    /*s05*/ register!(module);
}
fn helper() {}
fn callback(x: i32) -> i32 { x }
fn panic() {} // unrelated function, never a macro definition
fn direct() {
    /*s06*/ panic!("panic");
    /*s07*/ unreachable!("unreachable");
    /*s08*/ todo!("todo");
    /*s09*/ unimplemented!("unimplemented");
    /*s10*/ assert!("assert");
    /*s11*/ assert_eq!("assert_eq");
    /*s12*/ assert_ne!("assert_ne");
    /*s13*/ debug_assert!("debug_assert");
    /*s14*/ debug_assert_eq!("debug_assert_eq");
    /*s15*/ debug_assert_ne!("debug_assert_ne");
    /*s16*/ std::panic!("qualified");
    /*s17*/ core::unreachable!("qualified");
    /*s18*/ ::std::todo!("absolute");
    /*s19*/ r#panic!("raw");
    /*s20*/ std /* keep raw :: words */ :: r#assert_eq!(1, 2);
    let snow = "雪🙂";
    /*s21*/ panic!("utf8");
    /*s22*/ debug_assert!(
        true,
        "multiline"
    );
    /*s23*/ todo! { "brace" };
    /*s24*/ unimplemented!["bracket"];
    /*s25*/ panic!("repeat");
    /*s26*/ panic!("repeat");
    /*s27*/ explode!("alias");
    /*s28*/ wrapper!(panic!("not direct"));
    /*s29*/ stringify!(panic!("not direct either"));
    /*s30*/ json!({ "x": helper() });
    helper();
    let _ = [1].into_iter().map(callback);
    // panic!("comment")
    let _ = "todo!(string)";
    /*s31*/ panic!("before local");
    macro_rules! panic { ($($t:tt)*) => { panic!("definition body") }; }
    /*s32*/ panic!("local");
fn inner() {
    /*s33*/ debug_assert!("inner");
}
    const LC: () =     /*s34*/ panic!("local const");
    let _: [u8;     /*s35*/ size!()];
    const {
    /*s36*/ assert!(true);
    };
}
#[test]
fn test_fn() {
    /*s37*/ panic!("test");
}
#[tokio::test]
async fn async_fn() {
    /*s38*/ assert!("async");
}
#[cfg(test)]
mod tests {
#[cfg_attr(feature = "x", test)]
fn maybe() {
    /*s39*/ debug_assert!("conditional");
fn inner() {
    /*s40*/ panic!("nested conditional");
}
}
}
#[cfg_attr(feature = "x", cfg(test))]
mod conditional {
#[cfg(feature = "y")]
fn conditional_fn() {
    /*s41*/ todo!("cfg");
}
}
fn unrelated() {
    macro_rules! panic { () => { todo!("hidden") }; }
}
fn after() {
    /*s42*/ panic!("unrelated definition");
}
"###;

const EXPECTED: &str = r###"[
{"id": "s01", "text": "register!()", "path": "register", "normalized": "register", "scope": "file", "context": "unspecified", "resolution": "Unresolved", "sink": null, "start": 33, "end": 44, "attributes": ["#![allow(dead_code)]"], "caller": null, "kind": "file", "scope_range": [0, 2359], "phase": "item_syntax", "lines": [2, 2]},
{"id": "s02", "text": "type_macro!()", "path": "type_macro", "normalized": "type_macro", "scope": "alias", "context": "unspecified", "resolution": "Unresolved", "sink": null, "start": 71, "end": 84, "attributes": ["#![allow(dead_code)]"], "caller": null, "kind": "type", "scope_range": [46, 85], "phase": "item_syntax", "lines": [3, 3]},
{"id": "s03", "text": "panic!(\"const\")", "path": "panic", "normalized": "panic", "scope": "C", "context": "unspecified", "resolution": "Unresolved", "sink": "panic", "start": 112, "end": 127, "attributes": ["#![allow(dead_code)]"], "caller": null, "kind": "const", "scope_range": [86, 128], "phase": "const_context_syntax", "lines": [4, 4]},
{"id": "s04", "text": "todo!(\"static\")", "path": "todo", "normalized": "todo", "scope": "S", "context": "unspecified", "resolution": "Unresolved", "sink": "panic", "start": 156, "end": 171, "attributes": ["#![allow(dead_code)]"], "caller": null, "kind": "static", "scope_range": [129, 172], "phase": "const_context_syntax", "lines": [5, 5]},
{"id": "s05", "text": "register!(module)", "path": "register", "normalized": "register", "scope": "plain", "context": "unspecified", "resolution": "Unresolved", "sink": null, "start": 197, "end": 214, "attributes": ["#![allow(dead_code)]"], "caller": null, "kind": "module", "scope_range": [173, 217], "phase": "item_syntax", "lines": [7, 7]},
{"id": "s06", "text": "panic!(\"panic\")", "path": "panic", "normalized": "panic", "scope": "direct", "context": "unspecified", "resolution": "Unresolved", "sink": "panic", "start": 354, "end": 369, "attributes": ["#![allow(dead_code)]"], "caller": "direct", "kind": "function", "scope_range": [328, 1800], "phase": "function_body_syntax", "lines": [13, 13]},
{"id": "s07", "text": "unreachable!(\"unreachable\")", "path": "unreachable", "normalized": "unreachable", "scope": "direct", "context": "unspecified", "resolution": "Unresolved", "sink": "panic", "start": 383, "end": 410, "attributes": ["#![allow(dead_code)]"], "caller": "direct", "kind": "function", "scope_range": [328, 1800], "phase": "function_body_syntax", "lines": [14, 14]},
{"id": "s08", "text": "todo!(\"todo\")", "path": "todo", "normalized": "todo", "scope": "direct", "context": "unspecified", "resolution": "Unresolved", "sink": "panic", "start": 424, "end": 437, "attributes": ["#![allow(dead_code)]"], "caller": "direct", "kind": "function", "scope_range": [328, 1800], "phase": "function_body_syntax", "lines": [15, 15]},
{"id": "s09", "text": "unimplemented!(\"unimplemented\")", "path": "unimplemented", "normalized": "unimplemented", "scope": "direct", "context": "unspecified", "resolution": "Unresolved", "sink": "panic", "start": 451, "end": 482, "attributes": ["#![allow(dead_code)]"], "caller": "direct", "kind": "function", "scope_range": [328, 1800], "phase": "function_body_syntax", "lines": [16, 16]},
{"id": "s10", "text": "assert!(\"assert\")", "path": "assert", "normalized": "assert", "scope": "direct", "context": "unspecified", "resolution": "Unresolved", "sink": "assertion", "start": 496, "end": 513, "attributes": ["#![allow(dead_code)]"], "caller": "direct", "kind": "function", "scope_range": [328, 1800], "phase": "function_body_syntax", "lines": [17, 17]},
{"id": "s11", "text": "assert_eq!(\"assert_eq\")", "path": "assert_eq", "normalized": "assert_eq", "scope": "direct", "context": "unspecified", "resolution": "Unresolved", "sink": "assertion", "start": 527, "end": 550, "attributes": ["#![allow(dead_code)]"], "caller": "direct", "kind": "function", "scope_range": [328, 1800], "phase": "function_body_syntax", "lines": [18, 18]},
{"id": "s12", "text": "assert_ne!(\"assert_ne\")", "path": "assert_ne", "normalized": "assert_ne", "scope": "direct", "context": "unspecified", "resolution": "Unresolved", "sink": "assertion", "start": 564, "end": 587, "attributes": ["#![allow(dead_code)]"], "caller": "direct", "kind": "function", "scope_range": [328, 1800], "phase": "function_body_syntax", "lines": [19, 19]},
{"id": "s13", "text": "debug_assert!(\"debug_assert\")", "path": "debug_assert", "normalized": "debug_assert", "scope": "direct", "context": "unspecified", "resolution": "Unresolved", "sink": "debug_assertion", "start": 601, "end": 630, "attributes": ["#![allow(dead_code)]"], "caller": "direct", "kind": "function", "scope_range": [328, 1800], "phase": "function_body_syntax", "lines": [20, 20]},
{"id": "s14", "text": "debug_assert_eq!(\"debug_assert_eq\")", "path": "debug_assert_eq", "normalized": "debug_assert_eq", "scope": "direct", "context": "unspecified", "resolution": "Unresolved", "sink": "debug_assertion", "start": 644, "end": 679, "attributes": ["#![allow(dead_code)]"], "caller": "direct", "kind": "function", "scope_range": [328, 1800], "phase": "function_body_syntax", "lines": [21, 21]},
{"id": "s15", "text": "debug_assert_ne!(\"debug_assert_ne\")", "path": "debug_assert_ne", "normalized": "debug_assert_ne", "scope": "direct", "context": "unspecified", "resolution": "Unresolved", "sink": "debug_assertion", "start": 693, "end": 728, "attributes": ["#![allow(dead_code)]"], "caller": "direct", "kind": "function", "scope_range": [328, 1800], "phase": "function_body_syntax", "lines": [22, 22]},
{"id": "s16", "text": "std::panic!(\"qualified\")", "path": "std::panic", "normalized": "std::panic", "scope": "direct", "context": "unspecified", "resolution": "ExplicitPathCandidate", "sink": "panic", "start": 742, "end": 766, "attributes": ["#![allow(dead_code)]"], "caller": "direct", "kind": "function", "scope_range": [328, 1800], "phase": "function_body_syntax", "lines": [23, 23]},
{"id": "s17", "text": "core::unreachable!(\"qualified\")", "path": "core::unreachable", "normalized": "core::unreachable", "scope": "direct", "context": "unspecified", "resolution": "ExplicitPathCandidate", "sink": "panic", "start": 780, "end": 811, "attributes": ["#![allow(dead_code)]"], "caller": "direct", "kind": "function", "scope_range": [328, 1800], "phase": "function_body_syntax", "lines": [24, 24]},
{"id": "s18", "text": "::std::todo!(\"absolute\")", "path": "::std::todo", "normalized": "::std::todo", "scope": "direct", "context": "unspecified", "resolution": "ExplicitPathCandidate", "sink": "panic", "start": 825, "end": 849, "attributes": ["#![allow(dead_code)]"], "caller": "direct", "kind": "function", "scope_range": [328, 1800], "phase": "function_body_syntax", "lines": [25, 25]},
{"id": "s19", "text": "r#panic!(\"raw\")", "path": "r#panic", "normalized": "panic", "scope": "direct", "context": "unspecified", "resolution": "Unresolved", "sink": "panic", "start": 863, "end": 878, "attributes": ["#![allow(dead_code)]"], "caller": "direct", "kind": "function", "scope_range": [328, 1800], "phase": "function_body_syntax", "lines": [26, 26]},
{"id": "s20", "text": "std /* keep raw :: words */ :: r#assert_eq!(1, 2)", "path": "std /* keep raw :: words */ :: r#assert_eq", "normalized": "std::assert_eq", "scope": "direct", "context": "unspecified", "resolution": "ExplicitPathCandidate", "sink": "assertion", "start": 892, "end": 941, "attributes": ["#![allow(dead_code)]"], "caller": "direct", "kind": "function", "scope_range": [328, 1800], "phase": "function_body_syntax", "lines": [27, 27]},
{"id": "s21", "text": "panic!(\"utf8\")", "path": "panic", "normalized": "panic", "scope": "direct", "context": "unspecified", "resolution": "Unresolved", "sink": "panic", "start": 981, "end": 995, "attributes": ["#![allow(dead_code)]"], "caller": "direct", "kind": "function", "scope_range": [328, 1800], "phase": "function_body_syntax", "lines": [29, 29]},
{"id": "s22", "text": "debug_assert!(\n        true,\n        \"multiline\"\n    )", "path": "debug_assert", "normalized": "debug_assert", "scope": "direct", "context": "unspecified", "resolution": "Unresolved", "sink": "debug_assertion", "start": 1009, "end": 1063, "attributes": ["#![allow(dead_code)]"], "caller": "direct", "kind": "function", "scope_range": [328, 1800], "phase": "function_body_syntax", "lines": [30, 33]},
{"id": "s23", "text": "todo! { \"brace\" }", "path": "todo", "normalized": "todo", "scope": "direct", "context": "unspecified", "resolution": "Unresolved", "sink": "panic", "start": 1077, "end": 1094, "attributes": ["#![allow(dead_code)]"], "caller": "direct", "kind": "function", "scope_range": [328, 1800], "phase": "function_body_syntax", "lines": [34, 34]},
{"id": "s24", "text": "unimplemented![\"bracket\"]", "path": "unimplemented", "normalized": "unimplemented", "scope": "direct", "context": "unspecified", "resolution": "Unresolved", "sink": "panic", "start": 1108, "end": 1133, "attributes": ["#![allow(dead_code)]"], "caller": "direct", "kind": "function", "scope_range": [328, 1800], "phase": "function_body_syntax", "lines": [35, 35]},
{"id": "s25", "text": "panic!(\"repeat\")", "path": "panic", "normalized": "panic", "scope": "direct", "context": "unspecified", "resolution": "Unresolved", "sink": "panic", "start": 1147, "end": 1163, "attributes": ["#![allow(dead_code)]"], "caller": "direct", "kind": "function", "scope_range": [328, 1800], "phase": "function_body_syntax", "lines": [36, 36]},
{"id": "s26", "text": "panic!(\"repeat\")", "path": "panic", "normalized": "panic", "scope": "direct", "context": "unspecified", "resolution": "Unresolved", "sink": "panic", "start": 1177, "end": 1193, "attributes": ["#![allow(dead_code)]"], "caller": "direct", "kind": "function", "scope_range": [328, 1800], "phase": "function_body_syntax", "lines": [37, 37]},
{"id": "s27", "text": "explode!(\"alias\")", "path": "explode", "normalized": "explode", "scope": "direct", "context": "unspecified", "resolution": "Unresolved", "sink": null, "start": 1207, "end": 1224, "attributes": ["#![allow(dead_code)]"], "caller": "direct", "kind": "function", "scope_range": [328, 1800], "phase": "function_body_syntax", "lines": [38, 38]},
{"id": "s28", "text": "wrapper!(panic!(\"not direct\"))", "path": "wrapper", "normalized": "wrapper", "scope": "direct", "context": "unspecified", "resolution": "Unresolved", "sink": null, "start": 1238, "end": 1268, "attributes": ["#![allow(dead_code)]"], "caller": "direct", "kind": "function", "scope_range": [328, 1800], "phase": "function_body_syntax", "lines": [39, 39]},
{"id": "s29", "text": "stringify!(panic!(\"not direct either\"))", "path": "stringify", "normalized": "stringify", "scope": "direct", "context": "unspecified", "resolution": "Unresolved", "sink": null, "start": 1282, "end": 1321, "attributes": ["#![allow(dead_code)]"], "caller": "direct", "kind": "function", "scope_range": [328, 1800], "phase": "function_body_syntax", "lines": [40, 40]},
{"id": "s30", "text": "json!({ \"x\": helper() })", "path": "json", "normalized": "json", "scope": "direct", "context": "unspecified", "resolution": "Unresolved", "sink": null, "start": 1335, "end": 1359, "attributes": ["#![allow(dead_code)]"], "caller": "direct", "kind": "function", "scope_range": [328, 1800], "phase": "function_body_syntax", "lines": [41, 41]},
{"id": "s31", "text": "panic!(\"before local\")", "path": "panic", "normalized": "panic", "scope": "direct", "context": "unspecified", "resolution": "Unresolved", "sink": "panic", "start": 1484, "end": 1506, "attributes": ["#![allow(dead_code)]"], "caller": "direct", "kind": "function", "scope_range": [328, 1800], "phase": "function_body_syntax", "lines": [46, 46]},
{"id": "s32", "text": "panic!(\"local\")", "path": "panic", "normalized": "panic", "scope": "direct", "context": "unspecified", "resolution": "LocalDefinitionCandidate", "sink": "panic", "start": 1593, "end": 1608, "attributes": ["#![allow(dead_code)]"], "caller": "direct", "kind": "function", "scope_range": [328, 1800], "phase": "function_body_syntax", "lines": [48, 48]},
{"id": "s33", "text": "debug_assert!(\"inner\")", "path": "debug_assert", "normalized": "debug_assert", "scope": "inner", "context": "unspecified", "resolution": "Unresolved", "sink": "debug_assertion", "start": 1635, "end": 1657, "attributes": ["#![allow(dead_code)]"], "caller": "inner", "kind": "function", "scope_range": [1610, 1660], "phase": "function_body_syntax", "lines": [50, 50]},
{"id": "s34", "text": "panic!(\"local const\")", "path": "panic", "normalized": "panic", "scope": "LC", "context": "unspecified", "resolution": "LocalDefinitionCandidate", "sink": "panic", "start": 1692, "end": 1713, "attributes": ["#![allow(dead_code)]"], "caller": null, "kind": "const", "scope_range": [1665, 1714], "phase": "const_context_syntax", "lines": [52, 52]},
{"id": "s35", "text": "size!()", "path": "size", "normalized": "size", "scope": "array", "context": "unspecified", "resolution": "Unresolved", "sink": null, "start": 1743, "end": 1750, "attributes": ["#![allow(dead_code)]"], "caller": null, "kind": "type", "scope_range": [1726, 1751], "phase": "item_syntax", "lines": [53, 53]},
{"id": "s36", "text": "assert!(true)", "path": "assert", "normalized": "assert", "scope": "constblock", "context": "unspecified", "resolution": "Unresolved", "sink": "assertion", "start": 1777, "end": 1790, "attributes": ["#![allow(dead_code)]"], "caller": null, "kind": "const", "scope_range": [1757, 1797], "phase": "const_context_syntax", "lines": [55, 55]},
{"id": "s37", "text": "panic!(\"test\")", "path": "panic", "normalized": "panic", "scope": "test_fn", "context": "test_attribute_candidate", "resolution": "Unresolved", "sink": "panic", "start": 1836, "end": 1850, "attributes": ["#![allow(dead_code)]", "#[test]"], "caller": "test_fn", "kind": "function", "scope_range": [1809, 1853], "phase": "function_body_syntax", "lines": [60, 60]},
{"id": "s38", "text": "assert!(\"async\")", "path": "assert", "normalized": "assert", "scope": "async_fn", "context": "test_attribute_candidate", "resolution": "Unresolved", "sink": "assertion", "start": 1903, "end": 1919, "attributes": ["#![allow(dead_code)]", "#[tokio::test]"], "caller": "async_fn", "kind": "function", "scope_range": [1869, 1922], "phase": "function_body_syntax", "lines": [64, 64]},
{"id": "s39", "text": "debug_assert!(\"conditional\")", "path": "debug_assert", "normalized": "debug_assert", "scope": "maybe", "context": "conditional", "resolution": "Unresolved", "sink": "debug_assertion", "start": 2006, "end": 2034, "attributes": ["#![allow(dead_code)]", "#[cfg(test)]", "#[cfg_attr(feature = \"x\", test)]"], "caller": "maybe", "kind": "function", "scope_range": [1981, 2094], "phase": "function_body_syntax", "lines": [70, 70]},
{"id": "s40", "text": "panic!(\"nested conditional\")", "path": "panic", "normalized": "panic", "scope": "nested_test", "context": "conditional", "resolution": "Unresolved", "sink": "panic", "start": 2061, "end": 2089, "attributes": ["#![allow(dead_code)]", "#[cfg(test)]", "#[cfg_attr(feature = \"x\", test)]"], "caller": "inner", "kind": "function", "scope_range": [2036, 2092], "phase": "function_body_syntax", "lines": [72, 72]},
{"id": "s41", "text": "todo!(\"cfg\")", "path": "todo", "normalized": "todo", "scope": "conditional_fn", "context": "conditional", "resolution": "Unresolved", "sink": "panic", "start": 2209, "end": 2221, "attributes": ["#![allow(dead_code)]", "#[cfg_attr(feature = \"x\", cfg(test))]", "#[cfg(feature = \"y\")]"], "caller": "conditional_fn", "kind": "function", "scope_range": [2175, 2224], "phase": "function_body_syntax", "lines": [80, 80]},
{"id": "s42", "text": "panic!(\"unrelated definition\")", "path": "panic", "normalized": "panic", "scope": "after", "context": "unspecified", "resolution": "Unresolved", "sink": "panic", "start": 2325, "end": 2355, "attributes": ["#![allow(dead_code)]"], "caller": "after", "kind": "function", "scope_range": [2300, 2358], "phase": "function_body_syntax", "lines": [87, 87]}
]"###;

const SCOPES: &str = r###"{"file": {"kind": "file", "caller": null, "start": 0, "attributes": ["#![allow(dead_code)]"], "end": 2359}, "alias": {"kind": "type", "caller": null, "start": 46, "end": 85, "attributes": ["#![allow(dead_code)]"]}, "C": {"kind": "const", "caller": null, "start": 86, "end": 128, "attributes": ["#![allow(dead_code)]"]}, "S": {"kind": "static", "caller": null, "start": 129, "end": 172, "attributes": ["#![allow(dead_code)]"]}, "plain": {"kind": "module", "caller": null, "start": 173, "attributes": ["#![allow(dead_code)]"], "end": 217}, "direct": {"kind": "function", "caller": "direct", "start": 328, "attributes": ["#![allow(dead_code)]"], "end": 1800}, "inner": {"kind": "function", "caller": "inner", "start": 1610, "attributes": ["#![allow(dead_code)]"], "end": 1660}, "LC": {"kind": "const", "caller": null, "start": 1665, "end": 1714, "attributes": ["#![allow(dead_code)]"]}, "array": {"kind": "type", "caller": null, "start": 1726, "end": 1751, "attributes": ["#![allow(dead_code)]"]}, "constblock": {"kind": "const", "caller": null, "start": 1757, "attributes": ["#![allow(dead_code)]"], "end": 1797}, "test_fn": {"kind": "function", "caller": "test_fn", "start": 1809, "attributes": ["#![allow(dead_code)]", "#[test]"], "end": 1853}, "async_fn": {"kind": "function", "caller": "async_fn", "start": 1869, "attributes": ["#![allow(dead_code)]", "#[tokio::test]"], "end": 1922}, "tests": {"kind": "module", "caller": null, "start": 1936, "attributes": ["#![allow(dead_code)]", "#[cfg(test)]"], "end": 2096}, "maybe": {"kind": "function", "caller": "maybe", "start": 1981, "attributes": ["#![allow(dead_code)]", "#[cfg(test)]", "#[cfg_attr(feature = \"x\", test)]"], "end": 2094}, "nested_test": {"kind": "function", "caller": "inner", "start": 2036, "attributes": ["#![allow(dead_code)]", "#[cfg(test)]", "#[cfg_attr(feature = \"x\", test)]"], "end": 2092}, "conditional": {"kind": "module", "caller": null, "start": 2135, "attributes": ["#![allow(dead_code)]", "#[cfg_attr(feature = \"x\", cfg(test))]"], "end": 2226}, "conditional_fn": {"kind": "function", "caller": "conditional_fn", "start": 2175, "attributes": ["#![allow(dead_code)]", "#[cfg_attr(feature = \"x\", cfg(test))]", "#[cfg(feature = \"y\")]"], "end": 2224}, "unrelated": {"kind": "function", "caller": "unrelated", "start": 2227, "attributes": ["#![allow(dead_code)]"], "end": 2299}, "after": {"kind": "function", "caller": "after", "start": 2300, "attributes": ["#![allow(dead_code)]"], "end": 2358}}"###;

#[derive(Debug, Deserialize)]
struct Expected {
    id: String,
    text: String,
    path: String,
    normalized: String,
    caller: Option<String>,
    kind: MacroScopeKind,
    phase: MacroExecutionPhase,
    context: MacroTestContext,
    resolution: String,
    sink: Option<MacroSinkFamily>,
    start: usize,
    end: usize,
    lines: [usize; 2],
    scope: String,
    scope_range: [usize; 2],
    attributes: Vec<String>,
}

#[derive(Deserialize)]
struct ExpectedScope {
    kind: MacroScopeKind,
    start: usize,
    end: usize,
}

fn fixture(source: &str) -> GraphFacts {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("fixture.rs"), source).unwrap();
    build_facts(dir.path(), None).unwrap().0
}

#[test]
fn notebook_cells_do_not_emit_physical_file_macro_evidence() {
    for (index, source) in [
        "fn demo() { panic!(\"cell\"); }\n",
        "fn helper() {}\nfn demo() { helper(); panic!(\"cell\"); }\n",
    ]
    .into_iter()
    .enumerate()
    {
        let dir = tempfile::tempdir().unwrap();
        // Canonicalize both arguments: macOS /var and /private/var aliasing must
        // not turn a provenance regression into a relative-path setup failure.
        let root = dir.path().canonicalize().unwrap();
        let notebook = serde_json::json!({
            "nbformat": 4,
            "nbformat_minor": 5,
            "metadata": {"language_info": {"name": "rust"}},
            "cells": [{
                "cell_type": "code",
                "id": "rust-cell",
                "metadata": {},
                "execution_count": null,
                "outputs": [],
                "source": [source],
            }],
        });
        let bytes = serde_json::to_vec(&notebook).unwrap();
        let path = root.join("review.ipynb");
        std::fs::write(&path, &bytes).unwrap();
        let path = path.canonicalize().unwrap();
        let facts = spur_graph::extract_notebook_facts(&root, &path, &bytes).unwrap();

        // A real Rust file stays eligible even when its name resembles a cell
        // identity. Eligibility belongs to the source owner, not path spelling.
        let physical_dir = tempfile::tempdir().unwrap();
        let physical_path = "physical.ipynb#cell:rust-cell.rs";
        std::fs::write(physical_dir.path().join(physical_path), source).unwrap();
        let physical = build_facts(physical_dir.path(), None).unwrap().0;

        if let Ok(output) = std::env::var("M2_NOTEBOOK_EVIDENCE_DIR") {
            let output = std::path::Path::new(&output);
            std::fs::create_dir_all(output).unwrap();
            std::fs::write(
                output.join(format!("case-{index}.json")),
                serde_json::to_vec_pretty(&serde_json::json!({
                    "source": source,
                    "source_oid": git_blob_oid(source.as_bytes()),
                    "notebook_bytes": String::from_utf8(bytes.clone()).unwrap(),
                    "notebook_oid": git_blob_oid(&bytes),
                    "notebook_nodes": facts.nodes,
                    "notebook_edges": facts.edges,
                    "notebook_rows": facts.macro_invocations,
                    "physical_nodes": physical.nodes,
                    "physical_rows": physical.macro_invocations,
                }))
                .unwrap(),
            )
            .unwrap();
        }

        assert_eq!(
            facts
                .nodes
                .iter()
                .filter(|n| n.kind == NodeKind::File)
                .count(),
            1
        );
        assert_eq!(
            facts
                .nodes
                .iter()
                .filter(|n| n.kind == NodeKind::Cell)
                .count(),
            1
        );
        let demo = facts
            .nodes
            .iter()
            .find(|n| n.kind == NodeKind::Function && n.label == "demo")
            .unwrap();
        assert!(facts
            .edges
            .iter()
            .any(|e| e.relation == RelationKind::Contains
                && e.target_node_id == Some(demo.node_id)
                && facts
                    .nodes
                    .iter()
                    .any(|n| n.node_id == e.source_node_id && n.kind == NodeKind::Cell)));
        if source.contains("helper") {
            let helper = facts
                .nodes
                .iter()
                .find(|n| n.kind == NodeKind::Function && n.label == "helper")
                .unwrap();
            assert!(
                facts.edges.iter().any(|e| e.relation == RelationKind::Calls
                    && e.source_node_id == demo.node_id
                    && e.target_node_id == Some(helper.node_id)),
                "ordinary Rust notebook calls must survive the macro evidence guard"
            );
        }

        assert_eq!(physical.macro_invocations.len(), 1);
        let row = &physical.macro_invocations[0];
        assert_eq!(row.file_path, physical_path);
        assert_eq!(row.source_oid, git_blob_oid(source.as_bytes()));
        assert_eq!(
            &source[row.byte_range[0]..row.byte_range[1]],
            "panic!(\"cell\")"
        );
        assert_eq!(row.line_range, lines(source, row.byte_range));
        assert!(physical.nodes.iter().any(|n| n.kind == NodeKind::Function
            && n.label == "demo"
            && Some(&n.stable_key) == row.caller_stable_symbol_id.as_ref()));
        assert!(
            facts.macro_invocations.is_empty(),
            "NotebookCell spans cannot become physical-file evidence: {:?}",
            facts.macro_invocations
        );
    }
}

// Independently intersect whole line intervals with the covered bytes, including
// the file's final newline. Do not use tree-sitter's exclusive end_point.row.
fn lines(source: &str, [start, end]: [usize; 2]) -> [usize; 2] {
    let mut offset = 0;
    let mut covered = Vec::new();
    for (index, line) in source.split_inclusive('\n').enumerate() {
        let next = offset + line.len();
        if start.max(offset) < end.min(next) {
            covered.push(index + 1);
        }
        offset = next;
    }
    [covered[0], *covered.last().unwrap()]
}

fn expected_scopes(facts: &GraphFacts) -> BTreeMap<String, MacroScopeEvidence> {
    let expected: BTreeMap<String, ExpectedScope> = serde_json::from_str(SCOPES).unwrap();
    let oid = git_blob_oid(SOURCE.as_bytes());
    expected
        .into_iter()
        .map(|(name, scope)| {
            let range = [scope.start, scope.end];
            let line_range = lines(SOURCE, range);
            let id = facts
                .nodes
                .iter()
                .find_map(|node| {
                    let span = facts
                        .spans
                        .iter()
                        .find(|span| Some(span.span_id) == node.source_span_id)?;
                    (span.start_byte as usize == scope.start && span.end_byte as usize == scope.end)
                        .then(|| node.stable_key.clone())
                })
                .unwrap_or_else(|| {
                    // Explicit syntax-only evidence identity; this never adds a GraphNode.
                    let kind = serde_json::to_value(scope.kind).unwrap();
                    format!(
                        "macro-scope-v1:{}:{}",
                        kind.as_str().unwrap(),
                        macro_invocation_site_id("fixture.rs", &oid, range, line_range)
                    )
                });
            (
                name,
                MacroScopeEvidence {
                    scope: MacroScopeRef {
                        stable_symbol_id: id,
                        kind: scope.kind,
                    },
                    file_path: "fixture.rs".into(),
                    source_oid: oid.clone(),
                    byte_range: range,
                    line_range,
                },
            )
        })
        .collect()
}

#[test]
fn direct_sites_have_exact_spans_callers_and_conservative_context() {
    let facts = fixture(SOURCE);
    let expected: Vec<Expected> = serde_json::from_str(EXPECTED).unwrap();
    assert_eq!(expected.len(), 42);
    assert_eq!(facts.macro_invocations.len(), expected.len(), "all 42 annotated direct AST sites must be retained; nested token text is not an extra site");
    let scopes = expected_scopes(&facts);
    let oid = git_blob_oid(SOURCE.as_bytes());
    assert_eq!(oid, "8ddf1b0716584da082b1a45ee75257c3bdeb473f");
    let mut ids = BTreeSet::new();
    for e in &expected {
        assert_eq!(&SOURCE[e.start..e.end], e.text, "{} source oracle", e.id);
        assert_eq!(
            lines(SOURCE, [e.start, e.end]),
            e.lines,
            "{} line oracle",
            e.id
        );
        let row = facts
            .macro_invocations
            .iter()
            .find(|row| row.byte_range == [e.start, e.end])
            .unwrap_or_else(|| panic!("missing {}: {}", e.id, e.text));
        assert_eq!(row.file_path, "fixture.rs");
        assert_eq!(row.source_oid, oid);
        assert_eq!(
            row.site_id,
            macro_invocation_site_id("fixture.rs", &oid, [e.start, e.end], e.lines)
        );
        assert!(ids.insert(&row.site_id), "duplicate identity {}", e.id);
        assert_eq!(row.raw_macro_path, e.path, "{} raw path", e.id);
        assert_eq!(
            row.normalized_macro_path, e.normalized,
            "{} normalized path",
            e.id
        );
        assert_eq!(row.line_range, e.lines, "{} lines", e.id);
        assert_eq!(
            row.enclosing, scopes[&e.scope].scope,
            "{} exact enclosing scope",
            e.id
        );
        assert_eq!(scopes[&e.scope].byte_range, e.scope_range);
        let caller = e.caller.as_ref().map(|label| {
            let node = facts
                .nodes
                .iter()
                .find(|node| node.stable_key == scopes[&e.scope].scope.stable_symbol_id)
                .unwrap();
            assert_eq!(&node.label, label);
            assert!(matches!(node.kind, NodeKind::Function | NodeKind::Method));
            node.stable_key.clone()
        });
        assert_eq!(
            row.caller_stable_symbol_id, caller,
            "{} exact innermost caller",
            e.id
        );
        assert_eq!(row.enclosing.kind, e.kind);
        assert_eq!(row.execution_phase, e.phase, "{} phase", e.id);
        assert_eq!(row.test_context, e.context, "{} test context", e.id);
        assert_eq!(row.sink_family_candidate, e.sink, "{} sink candidate", e.id);
        assert_eq!(
            row.debug_assertions_candidate,
            e.sink == Some(MacroSinkFamily::DebugAssertion)
        );
        let raw_attributes: BTreeSet<_> =
            row.attributes.iter().map(|a| a.raw_text.as_str()).collect();
        assert_eq!(
            raw_attributes,
            e.attributes.iter().map(String::as_str).collect(),
            "{} ancestor attributes",
            e.id
        );
        for attribute in &row.attributes {
            assert_eq!(
                &SOURCE[attribute.byte_range[0]..attribute.byte_range[1]],
                attribute.raw_text
            );
            assert_eq!(attribute.line_range, lines(SOURCE, attribute.byte_range));
            let owner = scopes
                .values()
                .find(|scope| scope.scope == attribute.owner)
                .unwrap();
            assert!(
                owner.byte_range[0] <= row.byte_range[0]
                    && row.byte_range[1] <= owner.byte_range[1]
            );
            let expected_owner = if attribute.raw_text.starts_with("#!") {
                "file"
            } else if attribute.raw_text == "#[test]" {
                "test_fn"
            } else if attribute.raw_text == "#[tokio::test]" {
                "async_fn"
            } else if attribute.raw_text == "#[cfg(test)]" {
                "tests"
            } else if attribute.raw_text == "#[cfg_attr(feature = \"x\", test)]" {
                "maybe"
            } else if attribute.raw_text == "#[cfg(feature = \"y\")]" {
                "conditional_fn"
            } else {
                "conditional"
            };
            assert_eq!(
                attribute.owner, scopes[expected_owner].scope,
                "{} attribute owner",
                e.id
            );
        }
        match (&row.resolution, e.resolution.as_str()) {
            (MacroResolutionEvidence::Unresolved, "Unresolved")
            | (MacroResolutionEvidence::ExplicitPathCandidate, "ExplicitPathCandidate") => {}
            (
                MacroResolutionEvidence::LocalDefinitionCandidate { definition },
                "LocalDefinitionCandidate",
            ) => {
                assert_eq!(definition.file_path, "fixture.rs");
                assert_eq!(definition.source_oid, oid);
                assert_eq!(
                    &SOURCE[definition.byte_range[0]..definition.byte_range[1]],
                    "macro_rules! panic { ($($t:tt)*) => { panic!(\"definition body\") }; }"
                );
                assert_eq!(definition.line_range, lines(SOURCE, definition.byte_range));
                assert!(definition.byte_range[1] <= row.byte_range[0]);
            }
            other => panic!("{} resolution mismatch: {other:?}", e.id),
        }
    }
    let set = MacroInvocationEvidenceSet {
        schema_version: MACRO_INVOCATION_SCHEMA_VERSION.into(),
        extractor_version: "m2-fixture-v1".into(),
        graph_content_hash: "m2-fixture-snapshot".into(),
        rows: facts.macro_invocations.clone(),
    };
    let sources = [MacroEvidenceSource {
        file_path: "fixture.rs",
        bytes: SOURCE.as_bytes(),
    }];
    let scopes: Vec<_> = scopes.into_values().collect();
    set.validate("m2-fixture-snapshot", &sources, &scopes)
        .unwrap();
    let mut invalid = set.clone();
    invalid.rows.push(invalid.rows[0].clone());
    assert!(invalid
        .validate("m2-fixture-snapshot", &sources, &scopes)
        .is_err());
    invalid = set.clone();
    invalid.rows[5].caller_stable_symbol_id = Some("unrelated-function".into());
    assert!(invalid
        .validate("m2-fixture-snapshot", &sources, &scopes)
        .is_err());
    if let Ok(path) = std::env::var("M2_EVIDENCE_OUT") {
        std::fs::write(path, serde_json::to_vec_pretty(&serde_json::json!({
            "source": SOURCE, "source_oid": oid, "expected": serde_json::from_str::<serde_json::Value>(EXPECTED).unwrap(),
            "rows": set.rows, "scopes": scopes,
        })).unwrap()).unwrap();
    }
}

#[test]
fn direct_evidence_does_not_add_macro_calls_or_change_existing_edges() {
    let facts = fixture("fn panic() {} fn helper() {} fn callback(x: i32) -> i32 { x } fn run() { panic!(); helper(); [1].into_iter().map(callback); json!({ \"x\": helper() }); }");
    assert_eq!(facts.macro_invocations.len(), 2);
    let labels: Vec<_> = facts
        .edges
        .iter()
        .filter_map(|edge| {
            if edge.relation != RelationKind::Calls {
                return None;
            }
            edge.target_label.as_deref().or_else(|| {
                facts
                    .nodes
                    .iter()
                    .find(|n| Some(n.node_id) == edge.target_node_id)
                    .map(|n| n.label.as_str())
            })
        })
        .collect();
    assert!(labels.contains(&"helper"));
    assert!(!labels.contains(&"panic"));
    assert!(facts
        .edges
        .iter()
        .any(|edge| edge.relation == RelationKind::References
            && edge.target_label.as_deref() == Some("callback")));
}

#[test]
fn recovery_keeps_healthy_sites_without_impossible_spans() {
    let source = "fn damaged() { let = ; panic!(\"kept\"); } fn okay() { todo!(\"kept\"); } fn broken() { assert!(";
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_rust::LANGUAGE.into())
        .unwrap();
    let tree = parser.parse(source, None).unwrap();
    assert!(tree.root_node().has_error());
    let facts = fixture(source);
    let rows = &facts.macro_invocations;
    assert_eq!(
        rows.iter()
            .map(|r| &source[r.byte_range[0]..r.byte_range[1]])
            .collect::<Vec<_>>(),
        ["panic!(\"kept\")", "todo!(\"kept\")"],
        "recovered AST: {}",
        tree.root_node().to_sexp()
    );
    for row in rows {
        assert!(row.byte_range[0] < row.byte_range[1] && row.byte_range[1] <= source.len());
        assert_eq!(row.line_range, lines(source, row.byte_range));
    }
}

#[test]
fn exact_changed_bytes_rebind_identity_without_inventing_a_new_caller() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("fixture.rs");
    let before = "fn same() { panic!(\"before\"); }\n";
    let after = "fn same() { panic!(\"dirty 雪\"); }\n";
    std::fs::write(&path, before).unwrap();
    let old = build_facts(dir.path(), None).unwrap().0;
    std::fs::write(&path, after).unwrap();
    let new = build_facts(dir.path(), None).unwrap().0;
    assert_eq!(old.macro_invocations.len(), 1);
    assert_eq!(new.macro_invocations.len(), 1);
    let old = &old.macro_invocations[0];
    let new = &new.macro_invocations[0];
    assert_eq!(old.source_oid, git_blob_oid(before.as_bytes()));
    assert_eq!(new.source_oid, git_blob_oid(after.as_bytes()));
    assert_ne!(old.source_oid, new.source_oid);
    assert_ne!(old.site_id, new.site_id);
    assert_eq!(old.caller_stable_symbol_id, new.caller_stable_symbol_id);
    let again = build_facts(dir.path(), None).unwrap().0;
    assert_eq!(again.macro_invocations[0], *new);
}

#[test]
fn alias_and_attribute_tokens_do_not_establish_namespace_or_build_conditions() {
    let source = r#"use std::panic as explode;
#[allow(unused, reason = "cfg(test), test, debug_assertions")]
fn run() { explode!(); other!(); }
#[cfg_attr(feature = "x", cfg_attr(feature = "y", test))]
fn maybe() { ordinary!(); }
"#;
    let facts = fixture(source);
    let rows = &facts.macro_invocations;
    assert_eq!(rows.len(), 3);
    assert_eq!(rows[0].normalized_macro_path, "explode");
    for row in &rows[..2] {
        assert_eq!(row.resolution, MacroResolutionEvidence::Unresolved);
        assert_eq!(row.test_context, MacroTestContext::Unspecified);
        assert!(!row.debug_assertions_candidate);
        assert_eq!(row.sink_family_candidate, None);
        assert_eq!(row.attributes.len(), 1);
    }
    assert_eq!(rows[2].test_context, MacroTestContext::Conditional);
    assert!(!rows[2].debug_assertions_candidate);
}

#[test]
fn lexical_macro_candidates_respect_order_and_nested_scope() {
    let source = r#"macro_rules! panic { () => {} }
mod child {
    fn before() { panic!(); }
    macro_rules! panic { () => { hidden!(); } }
    fn after() { panic!(); std::panic!(); }
}
fn outside() { panic!(); }
"#;
    let facts = fixture(source);
    assert_eq!(facts.macro_invocations.len(), 4);
    for (index, definition_start) in [
        (0, 0),
        (1, source.find("    macro_rules!").unwrap() + 4),
        (3, 0),
    ] {
        let MacroResolutionEvidence::LocalDefinitionCandidate { definition } =
            &facts.macro_invocations[index].resolution
        else {
            panic!("site {index} must retain its lexical candidate");
        };
        assert_eq!(definition.byte_range[0], definition_start);
    }
    assert_eq!(
        facts.macro_invocations[2].resolution,
        MacroResolutionEvidence::ExplicitPathCandidate
    );
}
