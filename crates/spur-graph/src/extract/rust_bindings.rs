//! Physical-source lexical bindings for ordinary Rust function calls.
//! Macro token trees and receiver/type lookup stay on their existing paths.

use tree_sitter::Node;

use super::languages::DefinitionBinding;
use crate::NodeId;

// The shared Rust query omits generic callees. Add only free-call shapes;
// field-expression receivers keep their established extraction behavior.
pub(super) const GENERIC_FREE_CALLS: &str = r#"
(call_expression
  function: (generic_function
    function: [
      (identifier) @call.name
      (scoped_identifier
        path: (_) @call.scope
        name: (identifier) @call.name)
    ])) @call
"#;

enum ItemBinding {
    Absent,
    Target(Option<NodeId>),
    Ambiguous,
}

/// Strip free-call type arguments without changing receiver/type resolution.
pub(super) fn callee(call: Node<'_>) -> Option<Node<'_>> {
    let callee = call.child_by_field_name("function")?;
    if callee.kind() == "generic_function" {
        callee.child_by_field_name("function")
    } else {
        Some(callee)
    }
}

/// `None` defers to the existing resolver; `Some(None)` preserves an
/// unresolved call whose value binding cannot identify a function item.
pub(super) fn resolve(
    call: Node<'_>,
    source: &str,
    definitions: &[DefinitionBinding<'_>],
    relative_path: &str,
) -> Option<Option<NodeId>> {
    let callee = callee(call)?;
    if callee.kind() == "scoped_identifier" {
        let name = text(callee.child_by_field_name("name")?, source)?;
        let path = text(callee.child_by_field_name("path")?, source)?;
        let mut scope = module_scope(call)?;
        let mut segments = path.split("::").peekable();
        let lexical = !matches!(segments.peek().copied()?, "self" | "crate" | "super");
        match segments.peek().copied()? {
            "self" => {
                segments.next();
            }
            "crate" => {
                // A file's AST alone does not establish its crate-root module.
                if !matches!(
                    relative_path,
                    "lib.rs" | "main.rs" | "src/lib.rs" | "src/main.rs"
                ) {
                    return None;
                }
                while let Some(parent) = scope.parent() {
                    scope = parent;
                }
                segments.next();
            }
            _ => {}
        }
        for (index, segment) in segments.enumerate() {
            if segment == "super" {
                scope = module_scope(scope.parent()?)?;
            } else {
                // Only the first relative segment searches lexical scopes.
                // Explicit self/crate/super paths start at their module.
                let lexical = lexical && index == 0;
                let start = if lexical { call } else { scope };
                scope = match inline_module(start, segment, source, lexical) {
                    Ok(module) => module?,
                    // An unsupported alias/type shadows a candidate inline
                    // module: retain evidence instead of falling back to it.
                    Err(()) => return Some(None),
                };
            }
        }
        return match item(scope, name, source, definitions) {
            ItemBinding::Target(target) => Some(target),
            _ => None,
        };
    }
    if callee.kind() != "identifier" {
        return None;
    }
    let name = text(callee, source)?;
    let mut ancestor = call.parent();
    let mut crossed_function = false;
    while let Some(scope) = ancestor {
        match scope.kind() {
            "block" | "source_file" | "declaration_list" => {
                // A let binding's interval begins after the whole declaration,
                // excluding its initializer and let-else branch, and ends at
                // this block's end. A sibling block never contains this call.
                if !crossed_function {
                    let mut cursor = scope.walk();
                    if scope.named_children(&mut cursor).any(|node| {
                        node.kind() == "let_declaration"
                            && node.end_byte() <= call.start_byte()
                            && node
                                .child_by_field_name("pattern")
                                .is_some_and(|p| binds(p, name, source))
                    }) {
                        return Some(None);
                    }
                }
                // Imports and constructors occupy the value namespace too.
                // Their existing resolver owns target/provenance selection.
                let mut cursor = scope.walk();
                if scope.named_children(&mut cursor).any(|node| {
                    (node.kind() == "use_declaration" && imports(node, name, source))
                        || (node.kind() == "struct_item"
                            && node
                                .child_by_field_name("name")
                                .and_then(|n| text(n, source))
                                == Some(name))
                }) {
                    return None;
                }
                match item(scope, name, source, definitions) {
                    ItemBinding::Target(target) => return Some(target),
                    ItemBinding::Ambiguous => return None,
                    ItemBinding::Absent => {}
                }
                // Nested modules do not inherit the parent module's bare names.
                if scope.kind() == "source_file"
                    || scope.parent().is_some_and(|p| p.kind() == "mod_item")
                {
                    break;
                }
            }
            "function_item" | "closure_expression" if !crossed_function => {
                if scope
                    .child_by_field_name("parameters")
                    .is_some_and(|p| binds(p, name, source))
                {
                    return Some(None);
                }
                crossed_function = scope.kind() == "function_item";
            }
            "for_expression" if !crossed_function => {
                if scope
                    .child_by_field_name("body")
                    .is_some_and(|body| contains(body, call))
                    && scope
                        .child_by_field_name("pattern")
                        .is_some_and(|p| binds(p, name, source))
                {
                    return Some(None);
                }
            }
            "if_expression" | "while_expression" if !crossed_function => {
                if let Some(condition) = scope.child_by_field_name("condition") {
                    let in_body = scope
                        .child_by_field_name("consequence")
                        .or_else(|| scope.child_by_field_name("body"))
                        .is_some_and(|body| contains(body, call));
                    if (in_body || contains(condition, call))
                        && condition_binds(condition, call, in_body, name, source)
                    {
                        return Some(None);
                    }
                }
            }
            "match_arm" if !crossed_function => {
                if scope
                    .child_by_field_name("pattern")
                    .is_some_and(|p| binds(p, name, source))
                {
                    return Some(None);
                }
            }
            _ => {}
        }
        ancestor = scope.parent();
    }
    // Do not let the global name index resurrect an invisible local item or
    // a same-file module homonym. Imports without a local candidate defer.
    definitions
        .iter()
        .any(|d| {
            d.node.kind() == "function_item"
                && d.node
                    .child_by_field_name("name")
                    .and_then(|n| text(n, source))
                    == Some(name)
        })
        .then_some(None)
}

fn inline_module<'tree>(
    mut scope: Node<'tree>,
    name: &str,
    source: &str,
    lexical: bool,
) -> Result<Option<Node<'tree>>, ()> {
    let mut shadowed = false;
    loop {
        if let Some(parameters) = scope.child_by_field_name("type_parameters") {
            let mut cursor = parameters.walk();
            shadowed |= parameters.named_children(&mut cursor).any(|node| {
                node.kind() == "type_parameter"
                    && node
                        .child_by_field_name("name")
                        .and_then(|n| text(n, source))
                        == Some(name)
            });
        }
        if matches!(scope.kind(), "block" | "source_file" | "declaration_list") {
            let mut cursor = scope.walk();
            shadowed |= scope.named_children(&mut cursor).any(|node| {
                (node.kind() == "use_declaration" && imports(node, name, source))
                    || (matches!(
                        node.kind(),
                        "struct_item" | "enum_item" | "union_item" | "type_item" | "trait_item"
                    ) && node
                        .child_by_field_name("name")
                        .and_then(|n| text(n, source))
                        == Some(name))
            });
            let mut modules = scope.named_children(&mut cursor).filter(|node| {
                node.kind() == "mod_item"
                    && node
                        .child_by_field_name("name")
                        .and_then(|n| text(n, source))
                        == Some(name)
            });
            if let Some(module) = modules.next() {
                return if shadowed || modules.next().is_some() {
                    Err(())
                } else {
                    Ok(module.child_by_field_name("body"))
                };
            }
            if scope.kind() == "source_file"
                || scope.parent().is_some_and(|p| p.kind() == "mod_item")
            {
                break;
            }
        }
        if !lexical {
            break;
        }
        let Some(parent) = scope.parent() else {
            break;
        };
        scope = parent;
    }
    // No inline module candidate: keep the existing receiver/import resolver.
    Ok(None)
}

fn item(
    scope: Node<'_>,
    name: &str,
    source: &str,
    definitions: &[DefinitionBinding<'_>],
) -> ItemBinding {
    // Inherent/trait methods are not unqualified function items.
    if scope
        .parent()
        .is_some_and(|p| matches!(p.kind(), "impl_item" | "trait_item"))
    {
        return ItemBinding::Absent;
    }
    let mut cursor = scope.walk();
    if scope.named_children(&mut cursor).any(|node| {
        matches!(node.kind(), "const_item" | "static_item")
            && node
                .child_by_field_name("name")
                .and_then(|n| text(n, source))
                == Some(name)
    }) {
        return ItemBinding::Target(None);
    }
    let mut candidates = scope.named_children(&mut cursor).filter(|node| {
        node.kind() == "function_item"
            && node
                .child_by_field_name("name")
                .and_then(|n| text(n, source))
                == Some(name)
    });
    let Some(candidate) = candidates.next() else {
        return ItemBinding::Absent;
    };
    // cfg alternatives retain the existing resolver's duplicate-item policy.
    if candidates.next().is_some() {
        return ItemBinding::Ambiguous;
    }
    ItemBinding::Target(
        definitions
            .iter()
            .find(|d| d.node == candidate)
            .map(|d| d.node_id),
    )
}

fn module_scope(mut node: Node<'_>) -> Option<Node<'_>> {
    loop {
        if node.kind() == "source_file"
            || (node.kind() == "declaration_list"
                && node.parent().is_some_and(|p| p.kind() == "mod_item"))
        {
            return Some(node);
        }
        node = node.parent()?;
    }
}

fn binds(pattern: Node<'_>, name: &str, source: &str) -> bool {
    match pattern.kind() {
        "identifier" | "shorthand_field_identifier" => text(pattern, source) == Some(name),
        "parameter" => pattern
            .child_by_field_name("pattern")
            .is_some_and(|p| binds(p, name, source)),
        "field_pattern" => pattern
            .child_by_field_name("pattern")
            .or_else(|| pattern.child_by_field_name("name"))
            .is_some_and(|p| binds(p, name, source)),
        "scoped_identifier" | "type_identifier" | "generic_type" => false,
        _ => {
            let ty = pattern.child_by_field_name("type");
            let condition = pattern.child_by_field_name("condition");
            let mut cursor = pattern.walk();
            let found = pattern
                .named_children(&mut cursor)
                .any(|p| Some(p) != ty && Some(p) != condition && binds(p, name, source));
            found
        }
    }
}

fn imports(node: Node<'_>, name: &str, source: &str) -> bool {
    match node.kind() {
        "identifier" => text(node, source) == Some(name),
        "scoped_identifier" => {
            node.child_by_field_name("name")
                .and_then(|n| text(n, source))
                == Some(name)
        }
        "use_as_clause" => {
            node.child_by_field_name("alias")
                .and_then(|n| text(n, source))
                == Some(name)
        }
        "use_wildcard" => true,
        "scoped_use_list" => node
            .child_by_field_name("list")
            .is_some_and(|n| imports(n, name, source)),
        _ => {
            let mut cursor = node.walk();
            let found = node
                .named_children(&mut cursor)
                .any(|n| imports(n, name, source));
            found
        }
    }
}

fn condition_binds(
    condition: Node<'_>,
    call: Node<'_>,
    in_body: bool,
    name: &str,
    source: &str,
) -> bool {
    match condition.kind() {
        "let_condition" => {
            (in_body || condition.end_byte() <= call.start_byte())
                && condition
                    .child_by_field_name("pattern")
                    .is_some_and(|p| binds(p, name, source))
        }
        "let_chain" => {
            let mut cursor = condition.walk();
            let found = condition
                .named_children(&mut cursor)
                .any(|part| condition_binds(part, call, in_body, name, source));
            found
        }
        _ => false,
    }
}

fn text<'a>(node: Node<'_>, source: &'a str) -> Option<&'a str> {
    node.utf8_text(source.as_bytes()).ok()
}

fn contains(parent: Node<'_>, child: Node<'_>) -> bool {
    parent.start_byte() <= child.start_byte() && child.end_byte() <= parent.end_byte()
}
