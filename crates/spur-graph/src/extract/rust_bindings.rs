//! Physical-source lexical bindings for ordinary Rust calls.
//! Receiver lookup is deliberately limited to exact local nominal types.

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
    if callee.kind() == "field_expression" {
        // Own every ordinary receiver call, including unknown receivers. A
        // missing type must never fall through to a method-name singleton.
        return Some(resolve_method(callee, source, definitions));
    }
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

/// Export the exact local nominal path through the existing scope metadata.
/// This is source binding evidence, not compiler dispatch authority.
pub(super) fn receiver_scope(call: Node<'_>, source: &str) -> Option<String> {
    let callee = callee(call)?;
    if callee.kind() != "field_expression" {
        return None;
    }
    let ty = value_type(callee.child_by_field_name("value")?, source)?;
    let mut path = vec![text(ty.child_by_field_name("name")?, source)?];
    let mut parent = ty.parent();
    while let Some(node) = parent {
        if node.kind() == "mod_item" {
            path.push(text(node.child_by_field_name("name")?, source)?);
        }
        parent = node.parent();
    }
    path.reverse();
    Some(path.join("::"))
}

fn resolve_method(
    callee: Node<'_>,
    source: &str,
    definitions: &[DefinitionBinding<'_>],
) -> Option<NodeId> {
    let receiver = value_type(callee.child_by_field_name("value")?, source)?;
    // Nominal identity alone cannot establish Rust receiver candidate order.
    // A trait on T, &T, or a bounded blanket impl can precede an inherent
    // method after autoref/autoderef. Keep that whole dispatch unresolved.
    if trait_dispatch_uncertain(callee, receiver, source) {
        return None;
    }
    let name = text(callee.child_by_field_name("field")?, source)?;
    let mut candidates = definitions.iter().filter(|d| {
        if d.node.kind() != "function_item"
            || d.node
                .child_by_field_name("name")
                .and_then(|n| text(n, source))
                != Some(name)
        {
            return false;
        }
        let Some(implementation) = d.node.parent().and_then(|n| n.parent()) else {
            return false;
        };
        // Trait visibility, generic substitution, and autoderef need compiler
        // evidence. Only direct inherent methods participate in this lane.
        implementation.kind() == "impl_item"
            && implementation.child_by_field_name("trait").is_none()
            && implementation
                .child_by_field_name("type_parameters")
                .is_none()
            && implementation
                .child_by_field_name("type")
                .and_then(|ty| nominal_type(ty, source))
                == Some(receiver)
            && d.node
                .child_by_field_name("parameters")
                .is_some_and(|parameters| {
                    let mut cursor = parameters.walk();
                    let found = parameters
                        .named_children(&mut cursor)
                        .any(|p| p.kind() == "self_parameter");
                    found
                })
    });
    let target = candidates.next()?;
    if candidates.next().is_some() {
        return None;
    }
    if let Some(earlier_receiver) = prelude_method_may_precede_shared(name) {
        // The prelude is implicit: lack of physical impl/use nodes cannot
        // establish absence of blanket or reference implementations. The one
        // supported ordering proof is an owned nominal receiver and an
        // inherent &self method, with no prelude by-value/explicit receiver
        // of this name preceding &T. Inherent methods win at that candidate.
        let shared_self = target
            .node
            .child_by_field_name("parameters")
            .is_some_and(|parameters| {
                let mut cursor = parameters.walk();
                let found = parameters.named_children(&mut cursor).any(|parameter| {
                    parameter.kind() == "self_parameter"
                        && text(parameter, source)
                            .is_some_and(|s| s.split_whitespace().collect::<String>() == "&self")
                });
                found
            });
        if earlier_receiver
            || !shared_self
            || value_type_inner(callee.child_by_field_name("value")?, source, false)
                != Some(receiver)
        {
            return None;
        }
    }
    Some(target.node_id)
}

/// Union of receiver methods in the Rust 1.94.1 std/core preludes for editions
/// 2015/2018/2021/2024, including unstable methods conservatively. No edition
/// or prelude configuration is established by this physical AST. See R2C's
/// prelude-inventory.json for pinned compiler declarations and primary sources.
/// `true` includes by-value and explicit receivers (e.g. Future::poll); `false`
/// means every inventoried receiver of this name is &self or &mut self.
fn prelude_method_may_precede_shared(name: &str) -> Option<bool> {
    match name {
        "array_chunks" | "async_call_once" | "call_once" | "chain" | "clamp" | "cloned" | "cmp"
        | "cmp_by" | "collect" | "collect_into" | "copied" | "count" | "cycle" | "enumerate"
        | "eq" | "eq_by" | "filter" | "filter_map" | "flat_map" | "flatten" | "fold"
        | "for_each" | "fuse" | "ge" | "gt" | "inspect" | "intersperse" | "intersperse_with"
        | "into" | "into_future" | "into_iter" | "is_partitioned" | "is_sorted"
        | "is_sorted_by" | "is_sorted_by_key" | "last" | "le" | "lt" | "map" | "map_while"
        | "map_windows" | "max" | "max_by" | "max_by_key" | "min" | "min_by" | "min_by_key"
        | "ne" | "partial_cmp" | "partial_cmp_by" | "partition" | "partition_in_place"
        | "peekable" | "poll" | "product" | "reduce" | "rev" | "rfold" | "scan" | "skip"
        | "skip_while" | "step_by" | "sum" | "take" | "take_while" | "try_into" | "unzip"
        | "zip" => Some(true),
        "advance_back_by" | "advance_by" | "all" | "any" | "as_mut" | "as_ref" | "async_call"
        | "async_call_mut" | "by_ref" | "call" | "call_mut" | "clone" | "clone_from"
        | "clone_into" | "drop" | "extend" | "extend_one" | "extend_reserve" | "find"
        | "find_map" | "is_empty" | "len" | "next" | "next_back" | "next_chunk" | "nth"
        | "nth_back" | "position" | "rfind" | "rposition" | "size_hint" | "to_owned"
        | "to_string" | "try_collect" | "try_find" | "try_fold" | "try_for_each" | "try_reduce"
        | "try_rfold" => Some(false),
        _ => None,
    }
}

fn trait_dispatch_uncertain(call: Node<'_>, receiver: Node<'_>, source: &str) -> bool {
    // Imports may bring extension traits (including blanket impls) into scope.
    // Their visibility and implementations require evidence outside this AST.
    let mut scope = Some(call);
    while let Some(node) = scope {
        if matches!(node.kind(), "block" | "declaration_list" | "source_file") {
            let mut cursor = node.walk();
            if node
                .named_children(&mut cursor)
                .any(|child| child.kind() == "use_declaration")
            {
                return true;
            }
            if node.kind() == "source_file" || node.parent().is_some_and(|p| p.kind() == "mod_item")
            {
                break;
            }
        }
        scope = node.parent();
    }

    let mut root = call;
    while let Some(parent) = root.parent() {
        root = parent;
    }
    let mut pending = vec![root];
    while let Some(node) = pending.pop() {
        match node.kind() {
            // Another physical source file can implement a visible local or
            // prelude trait. This AST cannot certify its applicability.
            "mod_item" if node.child_by_field_name("body").is_none() => return true,
            // Expansions can introduce competing impls, even from a statement
            // or expression nested inside a function. This lane has no expanded
            // AST, so neither the macro name nor its tokens establish dispatch.
            "macro_invocation" => return true,
            "attribute_item" | "inner_attribute_item" => {
                let path = node
                    .named_child(0)
                    .and_then(|attribute| attribute.named_child(0));
                let non_generating = path
                    .filter(|path| path.kind() == "identifier")
                    .and_then(|path| text(path, source))
                    .is_some_and(|name| {
                        matches!(
                            name,
                            "allow" | "warn" | "deny" | "forbid" | "expect" | "doc"
                        )
                    });
                if !non_generating {
                    // Includes derives, procedural attributes, and cfg_attr.
                    return true;
                }
                // Lint/doc arguments are tokens, not ordinary Rust items.
                continue;
            }
            // A definition alone does not expand; token trees are never an
            // authoritative source of physical trait implementations.
            "macro_definition" | "token_tree" => continue,
            _ => {}
        }
        if node.kind() == "impl_item" && node.child_by_field_name("trait").is_some() {
            let implemented_type = node
                .child_by_field_name("type")
                .and_then(|ty| nominal_type(ty, source));
            // Erasing reference layers is deliberately an OVER-approximation
            // for rejection, never evidence to choose a method. Unknown types
            // include bounded generic, alias, and qualified trait impl owners.
            // Do not inspect only explicit impl methods: defaults/supertraits
            // can supply a competing receiver without a method body here.
            if implemented_type.is_none() || implemented_type == Some(receiver) {
                return true;
            }
        }
        let mut cursor = node.walk();
        pending.extend(node.named_children(&mut cursor));
    }
    false
}

/// Recover nominal identity, never just the last spelling of a type path.
/// References need no user-defined deref; aliases and generic types do.
fn nominal_type<'tree>(ty: Node<'tree>, source: &str) -> Option<Node<'tree>> {
    match ty.kind() {
        "reference_type" => nominal_type(ty.child_by_field_name("type")?, source),
        "type_identifier" | "identifier" => local_type(ty, text(ty, source)?, source),
        _ => None,
    }
}

fn local_type<'tree>(mut scope: Node<'tree>, name: &str, source: &str) -> Option<Node<'tree>> {
    loop {
        if scope
            .child_by_field_name("type_parameters")
            .is_some_and(|parameters| {
                let mut cursor = parameters.walk();
                let found = parameters.named_children(&mut cursor).any(|p| {
                    p.child_by_field_name("name").and_then(|n| text(n, source)) == Some(name)
                });
                found
            })
        {
            return None;
        }
        if matches!(scope.kind(), "block" | "declaration_list" | "source_file") {
            let mut cursor = scope.walk();
            let mut candidates = scope.named_children(&mut cursor).filter(|n| {
                (n.kind() == "use_declaration" && imports(*n, name, source))
                    || (matches!(
                        n.kind(),
                        "struct_item"
                            | "enum_item"
                            | "union_item"
                            | "type_item"
                            | "trait_item"
                            | "mod_item"
                    ) && n.child_by_field_name("name").and_then(|n| text(n, source))
                        == Some(name))
            });
            if let Some(candidate) = candidates.next() {
                return (candidates.next().is_none()
                    && matches!(candidate.kind(), "struct_item" | "enum_item" | "union_item"))
                .then_some(candidate);
            }
            // A child module does not inherit bare type names from its parent.
            if scope.kind() == "source_file"
                || scope.parent().is_some_and(|p| p.kind() == "mod_item")
            {
                return None;
            }
        }
        scope = scope.parent()?;
    }
}

fn value_type<'tree>(value: Node<'tree>, source: &str) -> Option<Node<'tree>> {
    value_type_inner(value, source, true)
}

// The dispatch-order witness uses the same lexical bindings as nominal scope,
// but rejects references instead of erasing them. A failed witness must never
// be interpreted as an owned receiver. `self` needs its own signature analysis.
fn value_type_inner<'tree>(
    value: Node<'tree>,
    source: &str,
    allow_references: bool,
) -> Option<Node<'tree>> {
    match value.kind() {
        "reference_expression" if allow_references => value_type_inner(
            value.child_by_field_name("value")?,
            source,
            allow_references,
        ),
        "parenthesized_expression" => {
            value_type_inner(value.named_child(0)?, source, allow_references)
        }
        "struct_expression" => nominal_type(value.child_by_field_name("name")?, source),
        "call_expression" => {
            let constructor = callee(value)?;
            let ty = nominal_type(constructor, source)?;
            // R1 owns value-namespace shadows too. With no definition IDs a
            // visible function/parameter/local is an unknown callable, never
            // evidence for the same-spelled tuple constructor.
            if resolve(value, source, &[], "").is_some() {
                return None;
            }
            // Tuple constructors only; ordinary function/associated-call return
            // inference is intentionally unsupported.
            (ty.kind() == "struct_item"
                && ty
                    .child_by_field_name("body")
                    .is_some_and(|body| body.kind() == "ordered_field_declaration_list"))
            .then_some(ty)
        }
        "identifier" => binding_type(value, text(value, source)?, source, allow_references),
        "self" if allow_references => {
            let mut scope = value.parent()?;
            loop {
                if scope.kind() == "impl_item" {
                    return nominal_type(scope.child_by_field_name("type")?, source);
                }
                scope = scope.parent()?;
            }
        }
        _ => None,
    }
}

fn binding_type<'tree>(
    value: Node<'tree>,
    name: &str,
    source: &str,
    allow_references: bool,
) -> Option<Node<'tree>> {
    let mut scope = value.parent()?;
    loop {
        match scope.kind() {
            "block" => {
                let mut cursor = scope.walk();
                // Lookup starts at the nearest completed declaration; the
                // initializer still sees the previous binding.
                let binding = scope
                    .named_children(&mut cursor)
                    .filter(|n| {
                        n.kind() == "let_declaration"
                            && n.end_byte() <= value.start_byte()
                            && n.child_by_field_name("pattern")
                                .is_some_and(|p| binds(p, name, source))
                    })
                    .last();
                if let Some(binding) = binding {
                    let pattern = binding.child_by_field_name("pattern")?;
                    if pattern.kind() != "identifier" {
                        return None;
                    }
                    return if let Some(ty) = binding.child_by_field_name("type") {
                        if !allow_references && ty.kind() == "reference_type" {
                            None
                        } else {
                            nominal_type(ty, source)
                        }
                    } else {
                        value_type_inner(
                            binding.child_by_field_name("value")?,
                            source,
                            allow_references,
                        )
                    };
                }
            }
            "function_item" | "closure_expression" => {
                if let Some(parameters) = scope.child_by_field_name("parameters") {
                    let mut cursor = parameters.walk();
                    let parameter = parameters
                        .named_children(&mut cursor)
                        .find(|p| binds(*p, name, source));
                    if let Some(parameter) = parameter {
                        let pattern = parameter.child_by_field_name("pattern")?;
                        if pattern.kind() != "identifier" {
                            return None;
                        }
                        let ty = parameter.child_by_field_name("type")?;
                        if !allow_references && ty.kind() == "reference_type" {
                            return None;
                        }
                        return nominal_type(ty, source);
                    }
                }
                if scope.kind() == "function_item" {
                    break;
                }
            }
            "for_expression" => {
                if scope
                    .child_by_field_name("body")
                    .is_some_and(|body| contains(body, value))
                    && scope
                        .child_by_field_name("pattern")
                        .is_some_and(|p| binds(p, name, source))
                {
                    return None;
                }
            }
            "match_arm" => {
                if scope
                    .child_by_field_name("pattern")
                    .is_some_and(|p| binds(p, name, source))
                {
                    return None;
                }
            }
            "if_expression" | "while_expression" => {
                if let Some(condition) = scope.child_by_field_name("condition") {
                    let in_body = scope
                        .child_by_field_name("consequence")
                        .or_else(|| scope.child_by_field_name("body"))
                        .is_some_and(|body| contains(body, value));
                    if (in_body || contains(condition, value))
                        && condition_binds(condition, value, in_body, name, source)
                    {
                        return None;
                    }
                }
            }
            "source_file" | "declaration_list" => break,
            _ => {}
        }
        scope = scope.parent()?;
    }
    let ty = local_type(value, name, source)?;
    // A bare unit constructor is exact only without a competing value item.
    let mut ancestor = value.parent();
    while let Some(scope) = ancestor {
        if matches!(scope.kind(), "block" | "source_file" | "declaration_list") {
            let mut cursor = scope.walk();
            if scope.named_children(&mut cursor).any(|n| {
                (matches!(n.kind(), "const_item" | "static_item" | "function_item")
                    && n.child_by_field_name("name").and_then(|n| text(n, source)) == Some(name))
                    || (n.kind() == "use_declaration" && imports(n, name, source))
            }) {
                return None;
            }
        }
        if scope == ty.parent()? {
            break;
        }
        ancestor = scope.parent();
    }
    (ty.kind() == "struct_item" && ty.child_by_field_name("body").is_none()).then_some(ty)
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
