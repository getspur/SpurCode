use std::collections::{BTreeSet, HashMap};

use tree_sitter::{Language as TsLanguage, Node};

use crate::extract::tree_sitter::{CaptureHit, ExtractedSymbol, FactBuilder, PendingEdge};
use crate::validation::compute_anchor_hash;
use crate::{FileId, GraphEdgeKind, NodeId, NodeKind, RelationKind};

#[derive(Debug)]
pub(crate) struct LanguageConfig {
    pub(crate) language: TsLanguage,
    pub(crate) inline_language: Option<TsLanguage>,
    pub(crate) queries: &'static [(&'static str, &'static str)],
    pub(crate) definition_kind_map: &'static [(&'static str, NodeKind)],
    pub(crate) relation_kind_map: Option<&'static [(&'static str, RelationKind)]>,
    pub(crate) preserve_bare_import_path: bool,
    pub(crate) is_method: Option<fn(Node<'_>) -> bool>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Language {
    Rust,
    Python,
    TypeScript,
    Tsx,
    Javascript,
    Markdown,
    JupyterNotebook,
    C,
    Cpp,
    Go,
    Hcl,
    Terraform,
    Lua,
    Shell,
    Sql,
    Json,
    Toml,
    Yaml,
    Mermaid,
    Html,
    Css,
}

/// Precision heuristic for common C runtime calls, not an exhaustive builtin list.
pub(crate) const C_BUILTIN_METHODS: &[&str] = &[
    "calloc", "close", "fclose", "fflush", "fopen", "fprintf", "fread", "free", "fwrite", "malloc",
    "memcpy", "memmove", "memset", "open", "printf", "read", "realloc", "snprintf", "sprintf",
    "strcmp", "strcpy", "strlen", "strncmp", "strncpy", "write",
];

/// Precision heuristic for common C++ standard-library methods, not an exhaustive builtin list.
pub(crate) const CPP_BUILTIN_METHODS: &[&str] = &[
    "append",
    "assign",
    "at",
    "back",
    "begin",
    "c_str",
    "capacity",
    "cbegin",
    "cend",
    "clear",
    "compare",
    "contains",
    "count",
    "data",
    "emplace_back",
    "empty",
    "end",
    "erase",
    "find",
    "first",
    "front",
    "insert",
    "length",
    "pop_back",
    "push_back",
    "rbegin",
    "rend",
    "reserve",
    "resize",
    "second",
    "size",
    "str",
    "substr",
    "swap",
];

/// Precision heuristic for common Go builtins, not an exhaustive builtin list.
pub(crate) const GO_BUILTIN_METHODS: &[&str] = &[
    "append", "cap", "close", "complex", "copy", "delete", "error", "len", "make", "new", "panic",
    "print", "println", "recover",
];

/// Precision heuristic for common Python standard-library methods, not an exhaustive builtin list.
pub(crate) const PYTHON_BUILTIN_METHODS: &[&str] = &[
    "add",
    "append",
    "clear",
    "close",
    "copy",
    "count",
    "decode",
    "discard",
    "encode",
    "endswith",
    "extend",
    "find",
    "format",
    "get",
    "index",
    "insert",
    "intersection",
    "items",
    "join",
    "keys",
    "lower",
    "lstrip",
    "pop",
    "popitem",
    "read",
    "recv",
    "remove",
    "replace",
    "reverse",
    "rsplit",
    "rstrip",
    "send",
    "setdefault",
    "sort",
    "split",
    "startswith",
    "strip",
    "union",
    "update",
    "upper",
    "values",
    "write",
];

/// Precision heuristic for common Rust std/core/trait methods, not an exhaustive builtin list.
pub(crate) const RUST_BUILTIN_METHODS: &[&str] = &[
    "and_then",
    "as_bytes",
    "as_mut",
    "as_path",
    "as_ref",
    "as_str",
    "borrow",
    "borrow_mut",
    "build",
    "bytes",
    "chars",
    "clear",
    "clone",
    "cmp",
    "collect",
    "contains",
    "contains_key",
    "count",
    "dedup",
    "default",
    "deref",
    "drain",
    "ends_with",
    "entry",
    "eq",
    "expect",
    "extend",
    "filter",
    "find",
    "first",
    "flush",
    "fmt",
    "from",
    "get",
    "get_mut",
    "insert",
    "into",
    "into_iter",
    "is_empty",
    "iter",
    "iter_mut",
    "join",
    "keys",
    "last",
    "len",
    "lines",
    "lock",
    "map",
    "max",
    "min",
    "next",
    "nth",
    "ok_or",
    "or_default",
    "or_insert",
    "parse",
    "pop",
    "push",
    "push_str",
    "read",
    "recv",
    "remove",
    "replace",
    "retain",
    "send",
    "sort",
    "split",
    "starts_with",
    "take",
    "to_lowercase",
    "to_owned",
    "to_path_buf",
    "to_string",
    "to_uppercase",
    "to_vec",
    "trim",
    "truncate",
    "unwrap",
    "unwrap_or",
    "values",
    "write",
];

/// Precision heuristic for common JS/TS runtime methods, not an exhaustive builtin list.
pub(crate) const TS_BUILTIN_METHODS: &[&str] = &[
    "add",
    "apply",
    "bind",
    "call",
    "catch",
    "charAt",
    "clear",
    "concat",
    "delete",
    "endsWith",
    "entries",
    "every",
    "filter",
    "finally",
    "find",
    "findIndex",
    "flat",
    "flatMap",
    "forEach",
    "get",
    "has",
    "hasOwnProperty",
    "includes",
    "indexOf",
    "join",
    "keys",
    "map",
    "match",
    "padEnd",
    "padStart",
    "pop",
    "push",
    "reduce",
    "repeat",
    "replace",
    "replaceAll",
    "reverse",
    "set",
    "shift",
    "slice",
    "some",
    "sort",
    "splice",
    "split",
    "startsWith",
    "substring",
    "then",
    "toLowerCase",
    "toString",
    "toUpperCase",
    "trim",
    "trimEnd",
    "trimStart",
    "unshift",
    "valueOf",
    "values",
];

/// Precision heuristic for common Lua runtime methods, not an exhaustive builtin list.
pub(crate) const LUA_BUILTIN_METHODS: &[&str] = &[
    "assert",
    "collectgarbage",
    "error",
    "format",
    "gmatch",
    "gsub",
    "insert",
    "ipairs",
    "len",
    "match",
    "next",
    "pairs",
    "pcall",
    "print",
    "remove",
    "require",
    "select",
    "sort",
    "sub",
    "tonumber",
    "tostring",
    "type",
    "unpack",
    "xpcall",
];

/// Precision heuristic for common shell builtins/commands, not an exhaustive builtin list.
pub(crate) const SHELL_BUILTIN_METHODS: &[&str] = &[
    ".", "[", "cat", "cd", "chmod", "cp", "echo", "env", "export", "find", "grep", "ln", "local",
    "mkdir", "mv", "printf", "pwd", "read", "rm", "sed", "set", "shift", "sort", "source", "test",
    "touch", "trap",
];

impl Language {
    pub fn from_path(path: &std::path::Path) -> Option<Self> {
        language_registry()
            .iter()
            .find_map(|descriptor| (descriptor.matcher)(path).then_some(descriptor.language))
    }

    pub fn tree_sitter_language(self) -> TsLanguage {
        match self {
            Self::Rust => tree_sitter_rust::LANGUAGE.into(),
            Self::Python => tree_sitter_python::LANGUAGE.into(),
            Self::TypeScript => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
            Self::Tsx => tree_sitter_typescript::LANGUAGE_TSX.into(),
            Self::Javascript => tree_sitter_typescript::LANGUAGE_TSX.into(),
            Self::Markdown => tree_sitter_md::LANGUAGE.into(),
            Self::JupyterNotebook => tree_sitter_md::LANGUAGE.into(),
            Self::C => tree_sitter_c::LANGUAGE.into(),
            Self::Cpp => tree_sitter_cpp::LANGUAGE.into(),
            Self::Go => tree_sitter_go::LANGUAGE.into(),
            Self::Hcl => tree_sitter_hcl::LANGUAGE.into(),
            Self::Terraform => tree_sitter_hcl::LANGUAGE.into(),
            Self::Lua => tree_sitter_lua::LANGUAGE.into(),
            Self::Shell => tree_sitter_bash::LANGUAGE.into(),
            Self::Sql => tree_sitter_sequel::LANGUAGE.into(),
            Self::Json => tree_sitter_json::LANGUAGE.into(),
            Self::Toml => tree_sitter_toml_ng::LANGUAGE.into(),
            Self::Yaml => tree_sitter_yaml::LANGUAGE.into(),
            Self::Mermaid => tree_sitter_mermaid::LANGUAGE.into(),
            Self::Html => tree_sitter_html::LANGUAGE.into(),
            Self::Css => tree_sitter_css::LANGUAGE.into(),
        }
    }

    pub(crate) fn config(self) -> LanguageConfig {
        match self {
            Self::Rust => rust_config(),
            Self::Python => python_config(),
            Self::TypeScript => typescript_config(),
            Self::Tsx => tsx_config(),
            Self::Javascript => javascript_config(),
            Self::Markdown => markdown_config(),
            Self::JupyterNotebook => jupyter_notebook_config(),
            Self::C => c_config(),
            Self::Cpp => cpp_config(),
            Self::Go => go_config(),
            Self::Hcl => hcl_config(),
            Self::Terraform => terraform_config(),
            Self::Lua => lua_config(),
            Self::Shell => shell_config(),
            Self::Sql => sql_config(),
            Self::Json => json_config(),
            Self::Toml => toml_config(),
            Self::Yaml => yaml_config(),
            Self::Mermaid => mermaid_config(),
            Self::Html => html_config(),
            Self::Css => css_config(),
        }
    }

    /// Common builtin/stdlib method names per language family: a precision heuristic, not exhaustive.
    pub(crate) fn builtin_method_names(self) -> &'static [&'static str] {
        match self {
            Self::Rust => RUST_BUILTIN_METHODS,
            Self::Python => PYTHON_BUILTIN_METHODS,
            Self::TypeScript | Self::Tsx | Self::Javascript => TS_BUILTIN_METHODS,
            Self::C => C_BUILTIN_METHODS,
            Self::Cpp => CPP_BUILTIN_METHODS,
            Self::Go => GO_BUILTIN_METHODS,
            Self::Lua => LUA_BUILTIN_METHODS,
            Self::Shell => SHELL_BUILTIN_METHODS,
            Self::Markdown
            | Self::JupyterNotebook
            | Self::Sql
            | Self::Hcl
            | Self::Terraform
            | Self::Json
            | Self::Toml
            | Self::Yaml
            | Self::Mermaid
            | Self::Html
            | Self::Css => &[],
        }
    }

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Rust => "rust",
            Self::Python => "python",
            Self::TypeScript => "typescript",
            Self::Tsx => "tsx",
            Self::Javascript => "javascript",
            Self::Markdown => "markdown",
            Self::JupyterNotebook => "jupyter_notebook",
            Self::C => "c",
            Self::Cpp => "cpp",
            Self::Go => "go",
            Self::Hcl => "hcl",
            Self::Terraform => "terraform",
            Self::Lua => "lua",
            Self::Shell => "shell",
            Self::Sql => "sql",
            Self::Json => "json",
            Self::Toml => "toml",
            Self::Yaml => "yaml",
            Self::Mermaid => "mermaid",
            Self::Html => "html",
            Self::Css => "css",
        }
    }
}

pub(crate) struct LanguageDescriptor {
    pub(crate) matcher: fn(&std::path::Path) -> bool,
    pub(crate) factory: fn() -> LanguageConfig,
    pub(crate) language: Language,
    pub(crate) label: &'static str,
    pub(crate) extensions: &'static [&'static str],
}

#[derive(Debug, Clone)]
pub(crate) struct DefinitionBinding<'tree> {
    pub(crate) node: Node<'tree>,
    pub(crate) node_id: NodeId,
    fqn: String,
}

pub(crate) fn rust_config() -> LanguageConfig {
    LanguageConfig {
        language: tree_sitter_rust::LANGUAGE.into(),
        inline_language: None,
        queries: &[
            ("tags", include_str!("../../queries/rust/tags.scm")),
            (
                "spur-edges",
                include_str!("../../queries/rust/spur-edges.scm"),
            ),
        ],
        definition_kind_map: &[
            ("definition.module", NodeKind::Module),
            ("definition.function", NodeKind::Function),
            ("definition.foreign_function", NodeKind::Function),
            ("definition.extern_block", NodeKind::ExternBlock),
            ("definition.method", NodeKind::Method),
            ("definition.struct", NodeKind::Struct),
            ("definition.enum", NodeKind::Enum),
            ("definition.enum_variant", NodeKind::EnumVariant),
            ("definition.tuple_field", NodeKind::Field),
            ("definition.field", NodeKind::Field),
            ("definition.trait", NodeKind::Trait),
            ("definition.impl", NodeKind::Impl),
            ("definition.type_alias", NodeKind::TypeAlias),
            ("definition.constant", NodeKind::Constant),
            ("definition.macro", NodeKind::Macro),
        ],
        relation_kind_map: None,
        preserve_bare_import_path: false,
        is_method: Some(has_impl_ancestor),
    }
}

pub(crate) fn python_config() -> LanguageConfig {
    LanguageConfig {
        language: tree_sitter_python::LANGUAGE.into(),
        inline_language: None,
        queries: &[
            ("tags", include_str!("../../queries/python/tags.scm")),
            (
                "spur-edges",
                include_str!("../../queries/python/spur-edges.scm"),
            ),
            (
                "spur-notebook-facts",
                include_str!("../../queries/python/spur-notebook-facts.scm"),
            ),
        ],
        definition_kind_map: &[
            ("definition.function", NodeKind::Function),
            ("definition.class", NodeKind::Class),
            ("definition.constant", NodeKind::Constant),
        ],
        relation_kind_map: None,
        preserve_bare_import_path: true,
        is_method: Some(has_class_definition_ancestor),
    }
}

// Shared by the plain TypeScript grammar. Must only reference nodes that exist
// in that grammar — JSX nodes are excluded here and added for TSX below.
const TYPESCRIPT_QUERIES: &[(&str, &str)] = &[
    ("tags", include_str!("../../queries/typescript/tags.scm")),
    (
        "spur-edges",
        include_str!("../../queries/typescript/spur-edges.scm"),
    ),
    (
        "spur-notebook-facts",
        include_str!("../../queries/typescript/spur-notebook-facts.scm"),
    ),
];

// TSX reuses the TypeScript tags/edges and adds JSX render edges. The JSX
// query references nodes that exist only in tree-sitter-typescript's TSX
// grammar, so it must never be compiled against the plain TypeScript grammar.
const TSX_QUERIES: &[(&str, &str)] = &[
    ("tags", include_str!("../../queries/typescript/tags.scm")),
    (
        "spur-edges",
        include_str!("../../queries/typescript/spur-edges.scm"),
    ),
    (
        "spur-notebook-facts",
        include_str!("../../queries/typescript/spur-notebook-facts.scm"),
    ),
    (
        "jsx-edges",
        include_str!("../../queries/typescript/jsx-edges.scm"),
    ),
];

const TYPESCRIPT_DEFINITION_KIND_MAP: &[(&str, NodeKind)] = &[
    ("definition.class", NodeKind::Class),
    ("definition.interface", NodeKind::Interface),
    ("definition.enum", NodeKind::Enum),
    ("definition.enum_variant", NodeKind::EnumVariant),
    ("definition.function", NodeKind::Function),
    ("definition.method", NodeKind::Method),
    ("definition.field", NodeKind::Field),
    ("definition.type_alias", NodeKind::TypeAlias),
    ("definition.constant", NodeKind::Constant),
    ("definition.module", NodeKind::Module),
];

fn typescript_config_for(
    language: TsLanguage,
    queries: &'static [(&'static str, &'static str)],
) -> LanguageConfig {
    LanguageConfig {
        language,
        inline_language: None,
        queries,
        definition_kind_map: TYPESCRIPT_DEFINITION_KIND_MAP,
        relation_kind_map: None,
        preserve_bare_import_path: false,
        is_method: None,
    }
}

pub(crate) fn typescript_config() -> LanguageConfig {
    typescript_config_for(
        tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
        TYPESCRIPT_QUERIES,
    )
}

pub(crate) fn tsx_config() -> LanguageConfig {
    typescript_config_for(tree_sitter_typescript::LANGUAGE_TSX.into(), TSX_QUERIES)
}

pub(crate) fn javascript_config() -> LanguageConfig {
    typescript_config_for(tree_sitter_typescript::LANGUAGE_TSX.into(), TSX_QUERIES)
}

pub(crate) fn c_config() -> LanguageConfig {
    LanguageConfig {
        language: tree_sitter_c::LANGUAGE.into(),
        inline_language: None,
        queries: &[
            ("tags", include_str!("../../queries/c/tags.scm")),
            ("spur-edges", include_str!("../../queries/c/spur-edges.scm")),
        ],
        definition_kind_map: &[
            ("definition.struct", NodeKind::Struct),
            ("definition.enum", NodeKind::Enum),
            ("definition.enum_variant", NodeKind::EnumVariant),
            ("definition.function", NodeKind::Function),
            ("definition.type_alias", NodeKind::TypeAlias),
            ("definition.macro", NodeKind::Macro),
            ("definition.field", NodeKind::Field),
            ("definition.constant", NodeKind::Constant),
        ],
        relation_kind_map: None,
        preserve_bare_import_path: false,
        is_method: None,
    }
}

pub(crate) fn cpp_config() -> LanguageConfig {
    LanguageConfig {
        language: tree_sitter_cpp::LANGUAGE.into(),
        inline_language: None,
        queries: &[
            ("tags", include_str!("../../queries/cpp/tags.scm")),
            (
                "spur-edges",
                include_str!("../../queries/cpp/spur-edges.scm"),
            ),
        ],
        definition_kind_map: &[
            ("definition.module", NodeKind::Module),
            ("definition.class", NodeKind::Class),
            ("definition.struct", NodeKind::Struct),
            ("definition.enum", NodeKind::Enum),
            ("definition.enum_variant", NodeKind::EnumVariant),
            ("definition.function", NodeKind::Function),
            ("definition.method", NodeKind::Method),
            ("definition.type_alias", NodeKind::TypeAlias),
            ("definition.macro", NodeKind::Macro),
            ("definition.field", NodeKind::Field),
            ("definition.constant", NodeKind::Constant),
        ],
        relation_kind_map: None,
        preserve_bare_import_path: false,
        is_method: Some(has_cpp_class_ancestor),
    }
}

pub(crate) fn go_config() -> LanguageConfig {
    LanguageConfig {
        language: tree_sitter_go::LANGUAGE.into(),
        inline_language: None,
        queries: &[
            ("tags", include_str!("../../queries/go/tags.scm")),
            (
                "spur-edges",
                include_str!("../../queries/go/spur-edges.scm"),
            ),
        ],
        definition_kind_map: &[
            ("definition.module", NodeKind::Module),
            ("definition.function", NodeKind::Function),
            ("definition.method", NodeKind::Method),
            ("definition.struct", NodeKind::Struct),
            ("definition.interface", NodeKind::Interface),
            ("definition.type_alias", NodeKind::TypeAlias),
            ("definition.constant", NodeKind::Constant),
            ("definition.field", NodeKind::Field),
        ],
        relation_kind_map: None,
        preserve_bare_import_path: true,
        is_method: None,
    }
}

const HCL_QUERIES: &[(&str, &str)] = &[
    ("tags", include_str!("../../queries/hcl/tags.scm")),
    (
        "spur-edges",
        include_str!("../../queries/hcl/spur-edges.scm"),
    ),
];

const HCL_DEFINITION_KIND_MAP: &[(&str, NodeKind)] = &[
    ("definition.resource", NodeKind::Resource),
    ("definition.data", NodeKind::Resource),
    ("definition.module", NodeKind::Resource),
    ("definition.variable", NodeKind::Constant),
    ("definition.output", NodeKind::Constant),
    ("definition.local", NodeKind::Constant),
];

fn hcl_config_for() -> LanguageConfig {
    LanguageConfig {
        language: tree_sitter_hcl::LANGUAGE.into(),
        inline_language: None,
        queries: HCL_QUERIES,
        definition_kind_map: HCL_DEFINITION_KIND_MAP,
        relation_kind_map: None,
        preserve_bare_import_path: false,
        is_method: None,
    }
}

pub(crate) fn hcl_config() -> LanguageConfig {
    hcl_config_for()
}

pub(crate) fn terraform_config() -> LanguageConfig {
    hcl_config_for()
}

pub(crate) fn lua_config() -> LanguageConfig {
    LanguageConfig {
        language: tree_sitter_lua::LANGUAGE.into(),
        inline_language: None,
        queries: &[
            ("tags", include_str!("../../queries/lua/tags.scm")),
            (
                "spur-edges",
                include_str!("../../queries/lua/spur-edges.scm"),
            ),
        ],
        definition_kind_map: &[
            ("definition.function", NodeKind::Function),
            ("definition.method", NodeKind::Method),
        ],
        relation_kind_map: None,
        preserve_bare_import_path: true,
        is_method: None,
    }
}

pub(crate) fn shell_config() -> LanguageConfig {
    LanguageConfig {
        language: tree_sitter_bash::LANGUAGE.into(),
        inline_language: None,
        queries: &[
            ("tags", include_str!("../../queries/shell/tags.scm")),
            (
                "spur-edges",
                include_str!("../../queries/shell/spur-edges.scm"),
            ),
        ],
        definition_kind_map: &[
            ("definition.function", NodeKind::Function),
            ("definition.constant", NodeKind::Constant),
        ],
        relation_kind_map: None,
        preserve_bare_import_path: true,
        is_method: None,
    }
}

pub(crate) fn sql_config() -> LanguageConfig {
    LanguageConfig {
        language: tree_sitter_sequel::LANGUAGE.into(),
        inline_language: None,
        queries: &[
            ("tags", include_str!("../../queries/sql/tags.scm")),
            (
                "spur-edges",
                include_str!("../../queries/sql/spur-edges.scm"),
            ),
        ],
        definition_kind_map: &[
            ("definition.struct", NodeKind::Struct),
            ("definition.type_alias", NodeKind::TypeAlias),
            ("definition.function", NodeKind::Function),
            ("definition.enum", NodeKind::Enum),
            ("definition.field", NodeKind::Field),
            ("definition.module", NodeKind::Module),
            ("definition.constant", NodeKind::Constant),
        ],
        relation_kind_map: None,
        preserve_bare_import_path: true,
        is_method: None,
    }
}

const STRUCTURED_DEFINITION_KIND_MAP: &[(&str, NodeKind)] = &[
    ("definition.module", NodeKind::Module),
    ("definition.field", NodeKind::Field),
    ("definition.constant", NodeKind::Constant),
];

const STRUCTURED_LOCKFILE_BASENAMES: &[&str] = &[
    "package-lock.json",
    "npm-shrinkwrap.json",
    "pnpm-lock.yaml",
    "pnpm-lock.yml",
    "yarn.lock",
    "bun.lock",
    "bun.lockb",
    "composer.lock",
    "poetry.lock",
    "uv.lock",
    "Pipfile.lock",
    "Gemfile.lock",
    "flake.lock",
];

fn is_structured_lockfile(path: &std::path::Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| {
            STRUCTURED_LOCKFILE_BASENAMES
                .iter()
                .any(|locked| name.eq_ignore_ascii_case(locked))
        })
}

fn json_config() -> LanguageConfig {
    LanguageConfig {
        language: tree_sitter_json::LANGUAGE.into(),
        inline_language: None,
        queries: &[("tags", include_str!("../../queries/json/tags.scm"))],
        definition_kind_map: STRUCTURED_DEFINITION_KIND_MAP,
        relation_kind_map: None,
        preserve_bare_import_path: false,
        is_method: None,
    }
}

fn toml_config() -> LanguageConfig {
    LanguageConfig {
        language: tree_sitter_toml_ng::LANGUAGE.into(),
        inline_language: None,
        queries: &[("tags", include_str!("../../queries/toml/tags.scm"))],
        definition_kind_map: STRUCTURED_DEFINITION_KIND_MAP,
        relation_kind_map: None,
        preserve_bare_import_path: false,
        is_method: None,
    }
}

fn yaml_config() -> LanguageConfig {
    LanguageConfig {
        language: tree_sitter_yaml::LANGUAGE.into(),
        inline_language: None,
        queries: &[("tags", include_str!("../../queries/yaml/tags.scm"))],
        definition_kind_map: STRUCTURED_DEFINITION_KIND_MAP,
        relation_kind_map: None,
        preserve_bare_import_path: false,
        is_method: None,
    }
}

const MERMAID_DEFINITION_KIND_MAP: &[(&str, NodeKind)] = &[("definition.module", NodeKind::Module)];

fn mermaid_config() -> LanguageConfig {
    LanguageConfig {
        language: tree_sitter_mermaid::LANGUAGE.into(),
        inline_language: None,
        queries: &[("tags", include_str!("../../queries/mermaid/tags.scm"))],
        definition_kind_map: MERMAID_DEFINITION_KIND_MAP,
        relation_kind_map: None,
        preserve_bare_import_path: false,
        is_method: None,
    }
}

const HTML_DEFINITION_KIND_MAP: &[(&str, NodeKind)] = &[("definition.section", NodeKind::Section)];

const CSS_DEFINITION_KIND_MAP: &[(&str, NodeKind)] = &[
    ("definition.section", NodeKind::Section),
    ("definition.function", NodeKind::Function),
    ("definition.constant", NodeKind::Constant),
];

fn html_config() -> LanguageConfig {
    LanguageConfig {
        language: tree_sitter_html::LANGUAGE.into(),
        inline_language: None,
        queries: &[
            ("tags", include_str!("../../queries/html/tags.scm")),
            (
                "spur-edges",
                include_str!("../../queries/html/spur-edges.scm"),
            ),
        ],
        definition_kind_map: HTML_DEFINITION_KIND_MAP,
        relation_kind_map: None,
        preserve_bare_import_path: true,
        is_method: None,
    }
}

fn css_config() -> LanguageConfig {
    LanguageConfig {
        language: tree_sitter_css::LANGUAGE.into(),
        inline_language: None,
        queries: &[
            ("tags", include_str!("../../queries/css/tags.scm")),
            (
                "spur-edges",
                include_str!("../../queries/css/spur-edges.scm"),
            ),
        ],
        definition_kind_map: CSS_DEFINITION_KIND_MAP,
        relation_kind_map: None,
        preserve_bare_import_path: true,
        is_method: None,
    }
}

pub(crate) fn markdown_config() -> LanguageConfig {
    LanguageConfig {
        language: tree_sitter_md::LANGUAGE.into(),
        inline_language: Some(tree_sitter_md::INLINE_LANGUAGE.into()),
        queries: &[
            ("tags", include_str!("../../queries/markdown/tags.scm")),
            (
                "spur-edges",
                include_str!("../../queries/markdown/spur-edges.scm"),
            ),
            (
                "inline-spur-edges",
                include_str!("../../queries/markdown/inline-spur-edges.scm"),
            ),
        ],
        definition_kind_map: &[("definition.section", NodeKind::Section)],
        relation_kind_map: Some(&[("import", RelationKind::Links)]),
        preserve_bare_import_path: false,
        is_method: None,
    }
}

pub(crate) fn jupyter_notebook_config() -> LanguageConfig {
    LanguageConfig {
        language: tree_sitter_md::LANGUAGE.into(),
        inline_language: None,
        queries: &[],
        definition_kind_map: &[],
        relation_kind_map: None,
        preserve_bare_import_path: false,
        is_method: None,
    }
}

fn markdown_matcher(path: &std::path::Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("md"))
}

fn jupyter_notebook_matcher(path: &std::path::Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("ipynb"))
}

fn rust_matcher(path: &std::path::Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("rs"))
}

fn python_matcher(path: &std::path::Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("py"))
}

fn typescript_matcher(path: &std::path::Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("ts"))
}

fn tsx_matcher(path: &std::path::Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("tsx"))
}

const JAVASCRIPT_EXTENSIONS: &[&str] = &["js", "jsx", "mjs", "cjs"];

fn javascript_matcher(path: &std::path::Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            JAVASCRIPT_EXTENSIONS
                .iter()
                .any(|candidate| extension.eq_ignore_ascii_case(candidate))
        })
}

const CPP_EXTENSIONS: &[&str] = &[
    "cpp", "cc", "cxx", "c++", "hpp", "hh", "hxx", "h++", "ipp", "tpp", "h",
];

fn cpp_matcher(path: &std::path::Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            CPP_EXTENSIONS
                .iter()
                .any(|candidate| extension.eq_ignore_ascii_case(candidate))
        })
}

fn c_matcher(path: &std::path::Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("c"))
}

fn go_matcher(path: &std::path::Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("go"))
}

fn hcl_matcher(path: &std::path::Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("hcl"))
}

fn terraform_matcher(path: &std::path::Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("tf"))
}

fn lua_matcher(path: &std::path::Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("lua"))
}

const SHELL_EXTENSIONS: &[&str] = &["sh", "bash", "zsh", "ksh"];

fn shell_matcher(path: &std::path::Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            SHELL_EXTENSIONS
                .iter()
                .any(|candidate| extension.eq_ignore_ascii_case(candidate))
        })
}

const SQL_EXTENSIONS: &[&str] = &["sql"];

fn sql_matcher(path: &std::path::Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            SQL_EXTENSIONS
                .iter()
                .any(|candidate| extension.eq_ignore_ascii_case(candidate))
        })
}

fn json_matcher(path: &std::path::Path) -> bool {
    !is_structured_lockfile(path)
        && path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("json"))
}

fn toml_matcher(path: &std::path::Path) -> bool {
    !is_structured_lockfile(path)
        && path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("toml"))
}

fn yaml_matcher(path: &std::path::Path) -> bool {
    !is_structured_lockfile(path)
        && path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| {
                extension.eq_ignore_ascii_case("yaml") || extension.eq_ignore_ascii_case("yml")
            })
}

fn mermaid_matcher(path: &std::path::Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            extension.eq_ignore_ascii_case("mmd") || extension.eq_ignore_ascii_case("mermaid")
        })
}

const HTML_EXTENSIONS: &[&str] = &["html", "htm"];

fn html_matcher(path: &std::path::Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            HTML_EXTENSIONS
                .iter()
                .any(|candidate| extension.eq_ignore_ascii_case(candidate))
        })
}

const CSS_EXTENSIONS: &[&str] = &["css"];

fn css_matcher(path: &std::path::Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            CSS_EXTENSIONS
                .iter()
                .any(|candidate| extension.eq_ignore_ascii_case(candidate))
        })
}

pub(crate) fn language_registry() -> &'static [LanguageDescriptor] {
    &[
        LanguageDescriptor {
            matcher: rust_matcher,
            factory: rust_config,
            language: Language::Rust,
            label: "rust",
            extensions: &["rs"],
        },
        LanguageDescriptor {
            matcher: python_matcher,
            factory: python_config,
            language: Language::Python,
            label: "python",
            extensions: &["py"],
        },
        LanguageDescriptor {
            matcher: typescript_matcher,
            factory: typescript_config,
            language: Language::TypeScript,
            label: "typescript",
            extensions: &["ts"],
        },
        LanguageDescriptor {
            matcher: tsx_matcher,
            factory: tsx_config,
            language: Language::Tsx,
            label: "tsx",
            extensions: &["tsx"],
        },
        LanguageDescriptor {
            matcher: javascript_matcher,
            factory: javascript_config,
            language: Language::Javascript,
            label: "javascript",
            extensions: JAVASCRIPT_EXTENSIONS,
        },
        LanguageDescriptor {
            matcher: c_matcher,
            factory: c_config,
            language: Language::C,
            label: "c",
            extensions: &["c"],
        },
        LanguageDescriptor {
            matcher: cpp_matcher,
            factory: cpp_config,
            language: Language::Cpp,
            label: "cpp",
            extensions: CPP_EXTENSIONS,
        },
        LanguageDescriptor {
            matcher: go_matcher,
            factory: go_config,
            language: Language::Go,
            label: "go",
            extensions: &["go"],
        },
        LanguageDescriptor {
            matcher: hcl_matcher,
            factory: hcl_config,
            language: Language::Hcl,
            label: "hcl",
            extensions: &["hcl"],
        },
        LanguageDescriptor {
            matcher: terraform_matcher,
            factory: terraform_config,
            language: Language::Terraform,
            label: "terraform",
            extensions: &["tf"],
        },
        LanguageDescriptor {
            matcher: lua_matcher,
            factory: lua_config,
            language: Language::Lua,
            label: "lua",
            extensions: &["lua"],
        },
        LanguageDescriptor {
            matcher: shell_matcher,
            factory: shell_config,
            language: Language::Shell,
            label: "shell",
            extensions: SHELL_EXTENSIONS,
        },
        LanguageDescriptor {
            matcher: sql_matcher,
            factory: sql_config,
            language: Language::Sql,
            label: "sql",
            extensions: SQL_EXTENSIONS,
        },
        LanguageDescriptor {
            matcher: markdown_matcher,
            factory: markdown_config,
            language: Language::Markdown,
            label: "markdown",
            extensions: &["md"],
        },
        LanguageDescriptor {
            matcher: jupyter_notebook_matcher,
            factory: jupyter_notebook_config,
            language: Language::JupyterNotebook,
            label: "jupyter_notebook",
            extensions: &["ipynb"],
        },
        LanguageDescriptor {
            matcher: json_matcher,
            factory: json_config,
            language: Language::Json,
            label: "json",
            extensions: &["json"],
        },
        LanguageDescriptor {
            matcher: toml_matcher,
            factory: toml_config,
            language: Language::Toml,
            label: "toml",
            extensions: &["toml"],
        },
        LanguageDescriptor {
            matcher: yaml_matcher,
            factory: yaml_config,
            language: Language::Yaml,
            label: "yaml",
            extensions: &["yaml", "yml"],
        },
        LanguageDescriptor {
            matcher: mermaid_matcher,
            factory: mermaid_config,
            language: Language::Mermaid,
            label: "mermaid",
            extensions: &["mmd", "mermaid"],
        },
        LanguageDescriptor {
            matcher: html_matcher,
            factory: html_config,
            language: Language::Html,
            label: "html",
            extensions: HTML_EXTENSIONS,
        },
        LanguageDescriptor {
            matcher: css_matcher,
            factory: css_config,
            language: Language::Css,
            label: "css",
            extensions: CSS_EXTENSIONS,
        },
    ]
}

pub fn all_supported_extensions() -> Vec<&'static str> {
    language_registry()
        .iter()
        .flat_map(|descriptor| descriptor.extensions.iter().copied())
        .collect()
}

pub(crate) fn emit_definitions<'tree>(
    config: &LanguageConfig,
    builder: &mut FactBuilder<'_>,
    relative_path: &str,
    file_id: FileId,
    file_node_id: NodeId,
    source: &str,
    captures: &[CaptureHit<'tree>],
) -> Vec<DefinitionBinding<'tree>> {
    emit_definitions_with_parents(
        config,
        builder,
        relative_path,
        file_id,
        file_node_id,
        source,
        captures,
        &[],
    )
}

#[expect(clippy::too_many_arguments)]
pub(crate) fn emit_definitions_with_parents<'tree>(
    config: &LanguageConfig,
    builder: &mut FactBuilder<'_>,
    relative_path: &str,
    file_id: FileId,
    file_node_id: NodeId,
    source: &str,
    captures: &[CaptureHit<'tree>],
    parent_bindings: &[DefinitionBinding<'tree>],
) -> Vec<DefinitionBinding<'tree>> {
    let definitions = definition_candidates(config, source, captures);

    let mut bindings = parent_bindings.to_vec();
    let mut emitted = Vec::new();
    for (kind, node, label, range) in definitions {
        let Some(label) = label else {
            continue;
        };
        let syntax_parent = nearest_parent(file_node_id, &bindings, node);
        let receiver_scope = go_method_receiver_scope(node, source);
        let receiver_parent = receiver_scope
            .as_deref()
            .and_then(|scope| parent_for_scope(&bindings, scope));
        let parent = receiver_parent.unwrap_or(syntax_parent);
        let fqn_prefix = receiver_parent
            .and_then(|parent| parent.fqn)
            .or(receiver_scope.as_deref())
            .or_else(|| {
                bindings
                    .iter()
                    .rev()
                    .find(|binding| {
                        binding.node.kind() != "foreign_mod_item" && contains(binding.node, node)
                    })
                    .map(|binding| binding.fqn.as_str())
            })
            .unwrap_or("");
        let fqn = scoped_name(fqn_prefix, &fqn_segment(kind, &label));
        let node_id =
            builder.add_node_with_range(relative_path, label, fqn.clone(), kind, file_id, range);
        builder.add_edge(parent.node_id, Some(node_id), RelationKind::Contains, None);
        if parent.node_id != file_node_id {
            builder.add_edge(parent.node_id, Some(node_id), RelationKind::Defines, None);
        }
        let binding = DefinitionBinding { node, node_id, fqn };
        bindings.push(binding.clone());
        emitted.push(binding);
    }
    emitted
}

pub(crate) fn extracted_symbols<'tree>(
    config: &LanguageConfig,
    source: &str,
    captures: &[CaptureHit<'tree>],
) -> Vec<ExtractedSymbol> {
    let definitions = definition_candidates(config, source, captures);
    let mut bindings: Vec<ExtractedDefinitionBinding<'tree>> = Vec::new();
    let mut symbols = Vec::new();

    for (kind, node, label, range) in definitions {
        let Some(label) = label else {
            continue;
        };
        let parent = nearest_extracted_parent(&bindings, node);
        let receiver_scope = go_method_receiver_scope(node, source);
        let enclosing_scope = receiver_scope
            .clone()
            .or_else(|| parent.map(|parent| parent.fqn.clone()));
        let fqn_prefix = receiver_scope
            .as_deref()
            .or_else(|| parent.map(|parent| parent.fqn.as_str()))
            .unwrap_or("");
        let fqn = scoped_name(fqn_prefix, &fqn_segment(kind, &label));
        let symbol_text = &source[range.start_byte..range.end_byte];
        symbols.push(ExtractedSymbol {
            entity_name: symbol_entity_name(&label),
            symbol_kind: symbol_kind(kind).to_owned(),
            enclosing_scope,
            byte_range: [range.start_byte, range.end_byte],
            line_range: [range.start_point.row + 1, range.end_point.row + 1],
            anchor_hash: compute_anchor_hash(symbol_text).to_string(),
            tokens: symbol_tokens(node, source, &label),
        });
        bindings.push(ExtractedDefinitionBinding { node, fqn });
    }

    symbols
}

pub(crate) fn emit_edges(
    config: &LanguageConfig,
    builder: &mut FactBuilder<'_>,
    file_node_id: NodeId,
    source: &str,
    definitions: &[DefinitionBinding<'_>],
    captures: &[CaptureHit<'_>],
) {
    let typed_bindings_by_scope = typed_bindings_by_scope(definitions, source);
    for capture in captures {
        match capture.name.as_str() {
            "import" => {
                let source_id = nearest_parent(file_node_id, definitions, capture.node).node_id;
                let relation = relation_kind_for_capture(config, "import", RelationKind::Imports);
                let import_paths = contained_capture_text(capture, source, captures, "import.path");
                for imported in contained_capture_text(capture, source, captures, "import.name") {
                    builder.pending_edges.push(PendingEdge {
                        source: source_id,
                        target_name: imported,
                        import_path: import_path_for_edge(
                            &import_paths,
                            config.preserve_bare_import_path,
                        ),
                        relation,
                        edge_kind: None,
                        origin: crate::extract::tree_sitter::CallOrigin::Expression,
                        receiver_text: None,
                        scope_text: None,
                    });
                }
            }
            "link" => {
                let source_id = nearest_parent(file_node_id, definitions, capture.node).node_id;
                for target in contained_capture_text(capture, source, captures, "link.name") {
                    if target.is_empty() || target.to_ascii_lowercase().starts_with("data:") {
                        continue;
                    }
                    builder.pending_edges.push(PendingEdge {
                        source: source_id,
                        target_name: target,
                        import_path: None,
                        relation: RelationKind::Links,
                        edge_kind: None,
                        origin: crate::extract::tree_sitter::CallOrigin::Expression,
                        receiver_text: None,
                        scope_text: None,
                    });
                }
            }
            "reexport" => {
                let source_id = nearest_parent(file_node_id, definitions, capture.node).node_id;
                let import_paths =
                    contained_capture_text(capture, source, captures, "reexport.path");
                let relation = relation_kind_for_capture(config, "reexport", RelationKind::Imports);
                for imported in contained_capture_text(capture, source, captures, "reexport.name") {
                    let edge = PendingEdge {
                        source: source_id,
                        target_name: imported,
                        import_path: import_path_for_edge(
                            &import_paths,
                            config.preserve_bare_import_path,
                        ),
                        relation,
                        edge_kind: None,
                        origin: crate::extract::tree_sitter::CallOrigin::Expression,
                        receiver_text: None,
                        scope_text: None,
                    };
                    builder.pending_edges.push(edge.clone());
                    builder.reexport_edges.push(edge);
                }
            }
            "implements" => {
                let source_id = nearest_parent(file_node_id, definitions, capture.node).node_id;
                for trait_name in
                    contained_capture_text(capture, source, captures, "implements.name")
                {
                    builder.pending_edges.push(PendingEdge {
                        source: source_id,
                        target_name: trait_name,
                        import_path: None,
                        relation: RelationKind::Implements,
                        edge_kind: None,
                        origin: crate::extract::tree_sitter::CallOrigin::Expression,
                        receiver_text: None,
                        scope_text: None,
                    });
                }
            }
            "extends" => {
                let source_id = nearest_parent(file_node_id, definitions, capture.node).node_id;
                for super_name in contained_capture_text(capture, source, captures, "extends.name")
                {
                    builder.pending_edges.push(PendingEdge {
                        source: source_id,
                        target_name: super_name,
                        import_path: None,
                        relation: RelationKind::Extends,
                        edge_kind: None,
                        origin: crate::extract::tree_sitter::CallOrigin::Expression,
                        receiver_text: None,
                        scope_text: None,
                    });
                }
            }
            "call" => {
                let source_id = nearest_parent(file_node_id, definitions, capture.node).node_id;
                let receiver_text =
                    contained_capture_text(capture, source, captures, "call.receiver")
                        .into_iter()
                        .next();
                let scope_text = contained_capture_text(capture, source, captures, "call.scope")
                    .into_iter()
                    .next();
                let inferred_scope_text = receiver_text.as_deref().and_then(|receiver| {
                    receiver_scope_text(source_id, receiver, &typed_bindings_by_scope)
                });
                let scope_text = scope_text.or(inferred_scope_text);
                for callee in contained_capture_text(capture, source, captures, "call.name") {
                    builder.pending_edges.push(PendingEdge {
                        source: source_id,
                        target_name: callee,
                        import_path: None,
                        relation: RelationKind::Calls,
                        edge_kind: None,
                        origin: crate::extract::tree_sitter::CallOrigin::Expression,
                        receiver_text: receiver_text.clone(),
                        scope_text: scope_text.clone(),
                    });
                }
            }
            "jsx_call" => {
                let source_id = nearest_parent(file_node_id, definitions, capture.node).node_id;
                for callee in contained_capture_text(capture, source, captures, "jsx_call.name") {
                    // React naming rule: a JSX tag whose name begins with an
                    // uppercase letter is a component reference; lowercase tags
                    // (`div`, `span`, …) are intrinsic host elements and must
                    // not produce call edges.
                    if !callee.chars().next().is_some_and(char::is_uppercase) {
                        continue;
                    }
                    builder.pending_edges.push(PendingEdge {
                        source: source_id,
                        target_name: callee,
                        import_path: None,
                        relation: RelationKind::Calls,
                        edge_kind: None,
                        origin: crate::extract::tree_sitter::CallOrigin::Expression,
                        receiver_text: None,
                        scope_text: None,
                    });
                }
            }
            "macro_call" => {
                let source_id = nearest_parent(file_node_id, definitions, capture.node).node_id;
                for arguments in captures.iter().filter(|hit| {
                    hit.name == "macro_call.args"
                        && hit.pattern_index == capture.pattern_index
                        && hit.match_index == capture.match_index
                }) {
                    let Some((callee, scope_text)) = rust_macro_callee(arguments.node, source)
                    else {
                        continue;
                    };
                    builder.pending_edges.push(PendingEdge {
                        source: source_id,
                        target_name: callee,
                        import_path: None,
                        relation: RelationKind::Calls,
                        edge_kind: None,
                        origin: crate::extract::tree_sitter::CallOrigin::MacroBody,
                        receiver_text: None,
                        scope_text,
                    });
                }
            }
            "reference.name" => {
                let source_id = nearest_parent(file_node_id, definitions, capture.node).node_id;
                let Ok(target_name) = capture.node.utf8_text(source.as_bytes()) else {
                    continue;
                };
                builder.pending_edges.push(PendingEdge {
                    source: source_id,
                    target_name: target_name.to_owned(),
                    import_path: None,
                    relation: RelationKind::References,
                    edge_kind: Some(GraphEdgeKind::ReferencesHof),
                    origin: crate::extract::tree_sitter::CallOrigin::Expression,
                    receiver_text: None,
                    scope_text: None,
                });
            }
            "hcl_ref" => {
                let Some(target_name) = hcl_reference_address(capture.node, source) else {
                    continue;
                };
                let source_id = nearest_parent(file_node_id, definitions, capture.node).node_id;
                builder.pending_edges.push(PendingEdge {
                    source: source_id,
                    target_name,
                    import_path: None,
                    relation: RelationKind::References,
                    edge_kind: Some(GraphEdgeKind::ReferencesAddress),
                    origin: crate::extract::tree_sitter::CallOrigin::Expression,
                    receiver_text: None,
                    scope_text: None,
                });
            }
            _ => {}
        }
    }
}

// Macro token trees expose path punctuation as siblings, not scoped_identifier
// fields. Anchor recovery at each argument list so mixed/nested calls cannot
// borrow a qualifier (or a turbofish type argument) from another callee.
fn rust_macro_callee(arguments: Node<'_>, source: &str) -> Option<(String, Option<String>)> {
    let mut callee = previous_macro_token(arguments)?;
    if matches!(callee.kind(), ">" | ">>") {
        let type_arguments = macro_type_arguments_start(callee)?;
        let separator = previous_macro_token(type_arguments)?;
        if separator.kind() != "::" {
            return None;
        }
        callee = previous_macro_token(separator)?;
    }
    if callee.kind() == "scoped_identifier" {
        let name = callee.child_by_field_name("name")?;
        let scope = callee
            .child_by_field_name("path")
            .map(|path| child_text(path, source).to_owned());
        return Some((child_text(name, source).to_owned(), scope));
    }
    if callee.kind() != "identifier" {
        return None;
    }
    let scope = previous_macro_token(callee)
        .filter(|separator| separator.kind() == "::")
        .map(|separator| {
            let start = previous_macro_token(separator)
                .and_then(macro_path_start)
                .unwrap_or(separator);
            // A leading `::callee()` is explicitly qualified too; do not turn
            // an empty path into permission for bare-name fallback.
            if start == separator {
                "::".to_owned()
            } else {
                source[start.start_byte()..separator.start_byte()]
                    .trim()
                    .to_owned()
            }
        });
    Some((child_text(callee, source).to_owned(), scope))
}

fn previous_macro_token(mut node: Node<'_>) -> Option<Node<'_>> {
    loop {
        node = node.prev_sibling()?;
        if !matches!(node.kind(), "line_comment" | "block_comment") {
            return Some(node);
        }
    }
}

fn macro_type_arguments_start(mut node: Node<'_>) -> Option<Node<'_>> {
    let mut depth = 0isize;
    loop {
        match node.kind() {
            ">" => depth += 1,
            ">>" => depth += 2,
            "<" => depth -= 1,
            "<<" => depth -= 2,
            _ => {}
        }
        if depth <= 0 {
            return (depth == 0).then_some(node);
        }
        node = previous_macro_token(node)?;
    }
}

fn macro_path_start(mut node: Node<'_>) -> Option<Node<'_>> {
    loop {
        if matches!(node.kind(), ">" | ">>") {
            node = macro_type_arguments_start(node)?;
            let Some(separator) = previous_macro_token(node).filter(|n| n.kind() == "::") else {
                // `<Type as Trait>` can itself be the qualifier.
                return Some(node);
            };
            node = previous_macro_token(separator)?;
        }
        if !matches!(
            node.kind(),
            "identifier" | "self" | "super" | "crate" | "scoped_identifier"
        ) {
            return None;
        }
        let Some(separator) = previous_macro_token(node).filter(|n| n.kind() == "::") else {
            return Some(node);
        };
        match previous_macro_token(separator) {
            Some(previous)
                if matches!(
                    previous.kind(),
                    "identifier" | "self" | "super" | "crate" | "scoped_identifier" | ">" | ">>"
                ) =>
            {
                node = previous;
            }
            _ => return Some(separator),
        }
    }
}

fn typed_bindings_by_scope(
    definitions: &[DefinitionBinding<'_>],
    source: &str,
) -> HashMap<NodeId, HashMap<String, String>> {
    let mut bindings_by_scope = HashMap::new();
    for definition in definitions {
        let mut bindings = HashMap::new();
        collect_typed_bindings(definition.node, definition.node, source, &mut bindings);
        if !bindings.is_empty() {
            bindings_by_scope.insert(definition.node_id, bindings);
        }
    }
    bindings_by_scope
}

fn collect_typed_bindings(
    root: Node<'_>,
    node: Node<'_>,
    source: &str,
    bindings: &mut HashMap<String, String>,
) {
    if node != root && is_definition_node(node) {
        return;
    }

    if matches!(node.kind(), "parameter" | "let_declaration") {
        if let Some((name, type_text)) = typed_binding(node, source) {
            bindings.insert(name, receiver_type_scope_text(&type_text));
        }
    }

    for index in 0..node.named_child_count() {
        if let Some(child) = node.named_child(index as u32) {
            collect_typed_bindings(root, child, source, bindings);
        }
    }
}

fn typed_binding(node: Node<'_>, source: &str) -> Option<(String, String)> {
    let pattern = node.child_by_field_name("pattern")?;
    let name = single_identifier_text(pattern, source)?;
    let type_node = node.child_by_field_name("type")?;
    Some((name, child_text(type_node, source).trim().to_owned()))
}

fn receiver_scope_text(
    source_id: NodeId,
    receiver: &str,
    typed_bindings_by_scope: &HashMap<NodeId, HashMap<String, String>>,
) -> Option<String> {
    let receiver = receiver.trim();
    if receiver == "self" {
        return Some("Self".to_owned());
    }
    typed_bindings_by_scope
        .get(&source_id)
        .and_then(|bindings| bindings.get(receiver))
        .cloned()
}

fn receiver_type_scope_text(type_text: &str) -> String {
    let mut ty = type_text.trim();
    while let Some(rest) = ty.strip_prefix('&') {
        ty = rest.trim_start();
        if let Some(rest) = ty.strip_prefix("mut ") {
            ty = rest.trim_start();
        }
        if let Some(rest) = strip_lifetime_prefix(ty) {
            ty = rest.trim_start();
            if let Some(rest) = ty.strip_prefix("mut ") {
                ty = rest.trim_start();
            }
        }
    }
    ty.to_owned()
}

fn strip_lifetime_prefix(type_text: &str) -> Option<&str> {
    let rest = type_text.strip_prefix('\'')?;
    let end = rest
        .find(char::is_whitespace)
        .map(|index| index + 1)
        .unwrap_or(type_text.len());
    Some(&type_text[end..])
}

pub(crate) fn emit_rust_dyn_trait_edges(
    builder: &mut FactBuilder<'_>,
    file_node_id: NodeId,
    source: &str,
    definitions: &[DefinitionBinding<'_>],
    root_node: Node<'_>,
) {
    let mut bindings_by_scope: HashMap<NodeId, HashMap<String, String>> = HashMap::new();
    for definition in definitions {
        let mut bindings = HashMap::new();
        collect_dyn_trait_bindings(definition.node, definition.node, source, &mut bindings);
        if !bindings.is_empty() {
            bindings_by_scope.insert(definition.node_id, bindings);
        }
    }

    let mut calls = Vec::new();
    collect_nodes_by_kind(root_node, "call_expression", &mut calls);
    for call in calls {
        let source_id = nearest_parent(file_node_id, definitions, call).node_id;
        let Some(bindings) = bindings_by_scope.get(&source_id) else {
            continue;
        };
        let Some((receiver, method)) = receiver_method_call(call, source) else {
            continue;
        };
        let Some(trait_name) = bindings.get(&receiver) else {
            continue;
        };
        builder.pending_edges.push(PendingEdge {
            source: source_id,
            target_name: format!("{trait_name}::{method}"),
            import_path: None,
            relation: RelationKind::Calls,
            edge_kind: Some(GraphEdgeKind::CallsDyn),
            origin: crate::extract::tree_sitter::CallOrigin::Expression,
            receiver_text: Some(receiver),
            scope_text: None,
        });
    }
}

fn collect_dyn_trait_bindings(
    root: Node<'_>,
    node: Node<'_>,
    source: &str,
    bindings: &mut HashMap<String, String>,
) {
    if node != root && is_definition_node(node) {
        return;
    }

    if matches!(node.kind(), "parameter" | "let_declaration") {
        if let Some((name, trait_name)) = dyn_trait_binding(node, source) {
            bindings.insert(name, trait_name);
        }
    }

    for index in 0..node.named_child_count() {
        if let Some(child) = node.named_child(index as u32) {
            collect_dyn_trait_bindings(root, child, source, bindings);
        }
    }
}

fn dyn_trait_binding(node: Node<'_>, source: &str) -> Option<(String, String)> {
    let pattern = node.child_by_field_name("pattern")?;
    let name = single_identifier_text(pattern, source)?;
    let type_node = node.child_by_field_name("type")?;
    let trait_name = dyn_trait_name_from_type(type_node, source)?;
    Some((name, trait_name))
}

fn dyn_trait_name_from_type(type_node: Node<'_>, source: &str) -> Option<String> {
    match type_node.kind() {
        "reference_type" => {
            let inner = type_node.child_by_field_name("type")?;
            (inner.kind() == "dynamic_type").then(|| trait_name_from_dynamic_type(inner, source))?
        }
        "generic_type" => {
            let base = type_node.child_by_field_name("type")?;
            let base = child_text(base, source).trim();
            if !matches!(base, "Box" | "Arc" | "Rc") {
                return None;
            }
            let type_arguments = type_node.child_by_field_name("type_arguments")?;
            let dynamic = direct_named_child_by_kind(type_arguments, "dynamic_type")?;
            trait_name_from_dynamic_type(dynamic, source)
        }
        _ => None,
    }
}

fn trait_name_from_dynamic_type(dynamic: Node<'_>, source: &str) -> Option<String> {
    let text = child_text(dynamic, source).trim();
    let rest = text.strip_prefix("dyn")?.trim_start();
    let primary = rest.split('+').next()?.trim();
    if primary.is_empty() || primary.contains(char::is_whitespace) || primary.contains('<') {
        return None;
    }
    Some(primary.to_owned())
}

fn direct_named_child_by_kind<'tree>(node: Node<'tree>, kind: &str) -> Option<Node<'tree>> {
    (0..node.named_child_count())
        .filter_map(|index| node.named_child(index as u32))
        .find(|child| child.kind() == kind)
}

fn receiver_method_call(call: Node<'_>, source: &str) -> Option<(String, String)> {
    let function = call.child_by_field_name("function")?;
    if function.kind() != "field_expression" {
        return None;
    }
    let receiver = function.child_by_field_name("value")?;
    if receiver.kind() != "identifier" {
        return None;
    }
    let method = function.child_by_field_name("field")?;
    if method.kind() != "field_identifier" {
        return None;
    }
    Some((
        child_text(receiver, source).trim().to_owned(),
        child_text(method, source).trim().to_owned(),
    ))
}

fn collect_nodes_by_kind<'tree>(node: Node<'tree>, kind: &str, nodes: &mut Vec<Node<'tree>>) {
    if node.kind() == kind {
        nodes.push(node);
    }
    for index in 0..node.named_child_count() {
        if let Some(child) = node.named_child(index as u32) {
            collect_nodes_by_kind(child, kind, nodes);
        }
    }
}

fn single_identifier_text(node: Node<'_>, source: &str) -> Option<String> {
    if node.kind() == "identifier" {
        return Some(child_text(node, source).trim().to_owned());
    }

    let identifiers = named_descendants_by_kind(node, "identifier");
    match identifiers.as_slice() {
        [identifier] => Some(child_text(*identifier, source).trim().to_owned()),
        _ => None,
    }
}

fn named_descendants_by_kind<'tree>(node: Node<'tree>, kind: &str) -> Vec<Node<'tree>> {
    let mut nodes = Vec::new();
    collect_nodes_by_kind(node, kind, &mut nodes);
    nodes
}

fn is_definition_node(node: Node<'_>) -> bool {
    matches!(
        node.kind(),
        "mod_item"
            | "struct_item"
            | "enum_item"
            | "trait_item"
            | "impl_item"
            | "function_item"
            | "function_signature_item"
    )
}

fn relation_kind_for_capture(
    config: &LanguageConfig,
    capture_name: &str,
    default: RelationKind,
) -> RelationKind {
    config
        .relation_kind_map
        .and_then(|map| {
            map.iter()
                .find_map(|(name, kind)| (*name == capture_name).then_some(*kind))
        })
        .unwrap_or(default)
}

fn definition_kind(
    config: &LanguageConfig,
    capture_name: &str,
    node: Node<'_>,
    source: &str,
) -> Option<NodeKind> {
    let mut kind = config
        .definition_kind_map
        .iter()
        .find_map(|(name, kind)| (*name == capture_name).then_some(*kind))?;
    if kind == NodeKind::Function && config.is_method.is_some_and(|is_method| is_method(node)) {
        kind = NodeKind::Method;
    } else if kind == NodeKind::Class && python_protocol_like_class(node, source) {
        kind = NodeKind::Interface;
    }
    Some(kind)
}

fn python_protocol_like_class(node: Node<'_>, source: &str) -> bool {
    if node.kind() != "class_definition" {
        return false;
    }

    let Some(superclasses) = node.child_by_field_name("superclasses") else {
        return false;
    };

    // Protocol and ABC are stdlib conventions, not definitions inside most
    // worktrees. Recognizing the local class declaration lets subclasses target
    // the local Protocol/ABC-like class as an Interface after resolution.
    (0..superclasses.named_child_count())
        .filter_map(|index| superclasses.named_child(index as u32))
        .any(|base| {
            matches!(
                child_text(base, source).trim(),
                "Protocol" | "ABC" | "abc.ABC"
            )
        })
}

fn definition_candidates<'tree>(
    config: &LanguageConfig,
    source: &str,
    captures: &[CaptureHit<'tree>],
) -> Vec<(NodeKind, Node<'tree>, Option<String>, tree_sitter::Range)> {
    let mut definitions: Vec<_> = captures
        .iter()
        .filter_map(|capture| {
            definition_kind(config, capture.name.as_str(), capture.node, source).map(|kind| {
                (
                    kind,
                    capture.node,
                    definition_name(kind, capture, source, captures),
                    if capture.name == "definition.tuple_field" {
                        tuple_field_range(capture.node)
                    } else if matches!(
                        capture.name.as_str(),
                        "definition.extern_block" | "definition.foreign_function"
                    ) {
                        foreign_declaration_range(capture.node)
                    } else {
                        capture.node.range()
                    },
                )
            })
        })
        .collect();

    definitions.sort_by_key(|(kind, node, _, _)| {
        (node.start_byte(), node.end_byte(), definition_rank(*kind))
    });
    definitions.dedup_by(|left, right| {
        left.1.start_byte() == right.1.start_byte()
            && left.1.end_byte() == right.1.end_byte()
            && left.1.kind() == right.1.kind()
    });
    definitions
}

fn tuple_field_range(node: Node<'_>) -> tree_sitter::Range {
    let mut range = node.range();
    let mut previous = node.prev_sibling();
    while let Some(sibling) = previous {
        if sibling.kind() == "," || sibling.kind() == "(" {
            break;
        }
        if matches!(sibling.kind(), "attribute_item" | "visibility_modifier")
            || (matches!(sibling.kind(), "line_comment" | "block_comment")
                && sibling.child_by_field_name("outer").is_some())
        {
            range.start_byte = sibling.start_byte();
            range.start_point = sibling.start_position();
        }
        previous = sibling.prev_sibling();
    }
    range
}

// Ordinary comments can separate an item's owned metadata. They remain in the
// full source slice between metadata and syntax, but never extend its start.
// Any preceding item or inner attribute ends ownership of outer metadata.
fn foreign_declaration_range(node: Node<'_>) -> tree_sitter::Range {
    let mut range = node.range();
    let mut previous = node.prev_named_sibling();
    while let Some(sibling) = previous {
        match sibling.kind() {
            "attribute_item" => {
                range.start_byte = sibling.start_byte();
                range.start_point = sibling.start_position();
            }
            "line_comment" | "block_comment" => {
                if sibling.child_by_field_name("outer").is_some() {
                    range.start_byte = sibling.start_byte();
                    range.start_point = sibling.start_position();
                }
            }
            _ => break,
        }
        previous = sibling.prev_named_sibling();
    }
    range
}

fn tuple_field_ordinal(node: Node<'_>) -> Option<usize> {
    let parent = node.parent()?;
    let mut ordinal = 0;
    for i in 0..parent.child_count() {
        let child = parent.child(i as u32)?;
        if parent.field_name_for_child(i as u32) == Some("type") {
            if child == node {
                return Some(ordinal);
            }
            ordinal += 1;
        }
    }
    None
}

fn definition_name(
    kind: NodeKind,
    definition: &CaptureHit<'_>,
    source: &str,
    captures: &[CaptureHit<'_>],
) -> Option<String> {
    if kind == NodeKind::Impl {
        let self_type = contained_capture_text(definition, source, captures, "impl.self")
            .into_iter()
            .next()?;
        let trait_type = contained_capture_text(definition, source, captures, "impl.trait")
            .into_iter()
            .next();
        return Some(match trait_type {
            Some(trait_type) => format!("{trait_type} for {self_type}"),
            None => self_type,
        });
    }

    match definition.name.as_str() {
        "definition.extern_block" => Some("<extern>".to_owned()),
        "definition.tuple_field" => tuple_field_ordinal(definition.node).map(|n| n.to_string()),
        "definition.resource" if is_hcl_provider_block(definition, source, captures) => {
            hcl_provider_address(definition, source, captures)
        }
        "definition.resource" if is_hcl_block(definition) => {
            hcl_block_address(definition, source, captures, None)
        }
        "definition.data" if is_hcl_block(definition) => {
            hcl_block_address(definition, source, captures, Some("data"))
        }
        "definition.module" if is_hcl_block(definition) => {
            hcl_label_address(definition, source, captures, "module")
        }
        "definition.variable" if is_hcl_block(definition) => {
            hcl_label_address(definition, source, captures, "var")
        }
        "definition.output" if is_hcl_block(definition) => {
            hcl_label_address(definition, source, captures, "output")
        }
        "definition.local" if definition.node.kind() == "attribute" => {
            contained_capture_text(definition, source, captures, "name")
                .into_iter()
                .next()
                .map(|name| format!("local.{name}"))
        }
        "definition.constant" if definition.node.kind() == "command" => {
            contained_capture_text(definition, source, captures, "name")
                .into_iter()
                .find_map(|name| shell_alias_name(&name))
        }
        _ => contained_capture_text(definition, source, captures, "name")
            .into_iter()
            .next()
            .map(|name| unquote_structured_key(&name)),
    }
}

fn unquote_structured_key(text: &str) -> String {
    let trimmed = text.trim();
    let bytes = trimmed.as_bytes();
    if bytes.len() >= 2 {
        let (first, last) = (bytes[0], bytes[bytes.len() - 1]);
        if (first == b'"' && last == b'"') || (first == b'\'' && last == b'\'') {
            return trimmed[1..trimmed.len() - 1].to_owned();
        }
    }
    trimmed.to_owned()
}

fn is_hcl_block(definition: &CaptureHit<'_>) -> bool {
    definition.node.kind() == "block"
}

fn hcl_block_address(
    definition: &CaptureHit<'_>,
    source: &str,
    captures: &[CaptureHit<'_>],
    prefix: Option<&str>,
) -> Option<String> {
    let block_type = hcl_label_text(definition, source, captures, "resource.type")?;
    let block_name = hcl_label_text(definition, source, captures, "resource.name")?;
    Some(match prefix {
        Some(prefix) => format!("{prefix}.{block_type}.{block_name}"),
        None => format!("{block_type}.{block_name}"),
    })
}

fn hcl_label_address(
    definition: &CaptureHit<'_>,
    source: &str,
    captures: &[CaptureHit<'_>],
    prefix: &str,
) -> Option<String> {
    let block_name = hcl_label_text(definition, source, captures, "resource.name")?;
    Some(format!("{prefix}.{block_name}"))
}

fn is_hcl_provider_block(
    definition: &CaptureHit<'_>,
    source: &str,
    captures: &[CaptureHit<'_>],
) -> bool {
    is_hcl_block(definition)
        && hcl_label_text(definition, source, captures, "provider.type").is_some()
}

fn hcl_provider_address(
    definition: &CaptureHit<'_>,
    source: &str,
    captures: &[CaptureHit<'_>],
) -> Option<String> {
    let provider_type = hcl_label_text(definition, source, captures, "provider.type")?;
    hcl_provider_alias(definition.node, source)
        .map(|alias| format!("{provider_type}.{alias}"))
        .or(Some(provider_type))
}

fn hcl_provider_alias(block: Node<'_>, source: &str) -> Option<String> {
    let body = direct_named_child_by_kind(block, "body")?;
    for index in 0..body.named_child_count() {
        let Some(attribute) = body.named_child(index as u32) else {
            continue;
        };
        if attribute.kind() != "attribute" {
            continue;
        }
        let Some(name) = direct_named_child_by_kind(attribute, "identifier") else {
            continue;
        };
        if child_text(name, source).trim() != "alias" {
            continue;
        }
        let literals = named_descendants_by_kind(attribute, "string_lit");
        match literals.as_slice() {
            [literal] => return string_lit_text(*literal, source),
            _ => return None,
        }
    }
    None
}

const HCL_RESERVED_REFERENCE_ROOTS: &[&str] =
    &["count", "each", "path", "provider", "self", "terraform"];

fn hcl_reference_address(node: Node<'_>, source: &str) -> Option<String> {
    let segments = hcl_reference_segments(node, source);
    let (first, rest) = segments.split_first()?;
    if rest.is_empty() || HCL_RESERVED_REFERENCE_ROOTS.contains(&first.as_str()) {
        return None;
    }
    let keep = match first.as_str() {
        "data" => 3,
        _ => 2,
    };
    if segments.len() < keep {
        return None;
    }
    Some(segments[..keep].join("."))
}

fn hcl_reference_segments(node: Node<'_>, source: &str) -> Vec<String> {
    let mut segments = Vec::new();
    let Some(identifier) = direct_named_child_by_kind(node, "identifier") else {
        return segments;
    };
    segments.push(child_text(identifier, source).trim().to_owned());
    let mut current = node.next_named_sibling();
    while let Some(sibling) = current {
        match sibling.kind() {
            "get_attr" => {
                if let Some(identifier) = direct_named_child_by_kind(sibling, "identifier") {
                    segments.push(child_text(identifier, source).trim().to_owned());
                }
            }
            "index" => {}
            _ => break,
        }
        current = sibling.next_named_sibling();
    }
    segments
}

fn hcl_label_text(
    parent: &CaptureHit<'_>,
    source: &str,
    captures: &[CaptureHit<'_>],
    capture_name: &str,
) -> Option<String> {
    captures
        .iter()
        .find(|capture| {
            capture.name == capture_name
                && capture.pattern_index == parent.pattern_index
                && capture.match_index == parent.match_index
        })
        .and_then(|capture| string_lit_text(capture.node, source))
}

fn shell_alias_name(text: &str) -> Option<String> {
    let name = text.split_once('=').map_or(text, |(name, _)| name).trim();
    (!name.is_empty()).then(|| name.to_owned())
}

fn parent_for_scope<'a>(bindings: &'a [DefinitionBinding<'_>], scope: &str) -> Option<Parent<'a>> {
    bindings
        .iter()
        .rev()
        .find(|binding| binding.fqn == scope)
        .map(|binding| Parent {
            node_id: binding.node_id,
            fqn: Some(&binding.fqn),
        })
}

fn go_method_receiver_scope(node: Node<'_>, source: &str) -> Option<String> {
    if node.kind() != "method_declaration" {
        return None;
    }
    let receiver = node.child_by_field_name("receiver")?;
    let declaration = direct_named_child_by_kind(receiver, "parameter_declaration")?;
    let receiver_type = declaration.child_by_field_name("type")?;
    Some(go_receiver_type_scope(receiver_type, source))
}

fn go_receiver_type_scope(mut node: Node<'_>, source: &str) -> String {
    while node.kind() == "pointer_type" {
        let Some(child) = node.named_child(0u32) else {
            break;
        };
        node = child;
    }
    child_text(node, source).trim().to_owned()
}

fn nearest_parent<'a>(
    file_node_id: NodeId,
    definitions: &'a [DefinitionBinding<'_>],
    node: Node<'_>,
) -> Parent<'a> {
    definitions
        .iter()
        .rev()
        .find(|definition| contains(definition.node, node))
        .map(|definition| Parent {
            node_id: definition.node_id,
            fqn: Some(&definition.fqn),
        })
        .unwrap_or(Parent {
            node_id: file_node_id,
            fqn: None,
        })
}

#[derive(Debug, Clone, Copy)]
struct Parent<'a> {
    node_id: NodeId,
    fqn: Option<&'a str>,
}

fn contains(parent: Node<'_>, child: Node<'_>) -> bool {
    parent.start_byte() <= child.start_byte() && child.end_byte() <= parent.end_byte()
}

fn has_impl_ancestor(node: Node<'_>) -> bool {
    let Some(parent) = node.parent() else {
        return false;
    };
    let Some(grandparent) = parent.parent() else {
        return false;
    };

    parent.kind() == "declaration_list" && grandparent.kind() == "impl_item"
}

fn has_cpp_class_ancestor(node: Node<'_>) -> bool {
    // A C++ `function_definition` is a method when it appears inside a class
    // or struct body (i.e. its enclosing `field_declaration_list` belongs to
    // a `class_specifier` / `struct_specifier` / `union_specifier`).
    // tree-sitter-cpp's `function_definition` for in-class methods nests
    // inside `field_declaration_list`. Walk upwards looking for that pairing.
    let mut current = node.parent();
    while let Some(parent) = current {
        if parent.kind() == "field_declaration_list" {
            if let Some(grandparent) = parent.parent() {
                return matches!(
                    grandparent.kind(),
                    "class_specifier" | "struct_specifier" | "union_specifier"
                );
            }
            return false;
        }
        // Stop walking once we hit a containing definition that isn't a class body.
        if matches!(
            parent.kind(),
            "namespace_definition" | "translation_unit" | "function_definition"
        ) {
            return false;
        }
        current = parent.parent();
    }
    false
}

fn has_class_definition_ancestor(node: Node<'_>) -> bool {
    let Some(parent) = node.parent() else {
        return false;
    };
    if let Some(grandparent) = parent.parent() {
        if parent.kind() == "block" && grandparent.kind() == "class_definition" {
            return true;
        }
    }

    if parent.kind() == "decorated_definition" {
        let Some(grandparent) = parent.parent() else {
            return false;
        };
        let Some(great_grandparent) = grandparent.parent() else {
            return false;
        };
        return grandparent.kind() == "block" && great_grandparent.kind() == "class_definition";
    }

    false
}

fn definition_rank(kind: NodeKind) -> u8 {
    match kind {
        NodeKind::Module => 0,
        NodeKind::Struct => 1,
        NodeKind::Class => 2,
        NodeKind::Interface => 3,
        NodeKind::Enum => 4,
        NodeKind::EnumVariant => 5,
        NodeKind::Trait => 6,
        NodeKind::Impl => 7,
        NodeKind::Method => 8,
        NodeKind::Function => 9,
        NodeKind::Section => 10,
        _ => 10,
    }
}

#[derive(Debug, Clone)]
struct ExtractedDefinitionBinding<'tree> {
    node: Node<'tree>,
    fqn: String,
}

fn nearest_extracted_parent<'a, 'tree>(
    definitions: &'a [ExtractedDefinitionBinding<'tree>],
    node: Node<'_>,
) -> Option<&'a ExtractedDefinitionBinding<'tree>> {
    definitions.iter().rev().find(|definition| {
        definition.node.kind() != "foreign_mod_item" && contains(definition.node, node)
    })
}

fn symbol_kind(kind: NodeKind) -> &'static str {
    match kind {
        NodeKind::Module => "module",
        NodeKind::ExternBlock => "extern_block",
        NodeKind::Function => "function",
        NodeKind::Class => "class",
        NodeKind::Interface => "interface",
        NodeKind::Struct => "struct",
        NodeKind::Impl => "impl",
        NodeKind::Trait => "trait",
        NodeKind::Enum => "enum",
        NodeKind::EnumVariant => "enum_variant",
        NodeKind::Method => "method",
        NodeKind::Field => "field",
        NodeKind::Constant => "constant",
        NodeKind::TypeAlias => "type_alias",
        NodeKind::Macro => "macro",
        NodeKind::Section => "section",
        NodeKind::Resource => "resource",
        _ => "symbol",
    }
}

fn symbol_entity_name(label: &str) -> String {
    label.strip_prefix("impl ").unwrap_or(label).to_owned()
}

fn fqn_segment(kind: NodeKind, label: &str) -> String {
    match kind {
        NodeKind::Impl => format!("impl {}", symbol_entity_name(label)),
        _ => symbol_entity_name(label),
    }
}

fn symbol_tokens(node: Node<'_>, source: &str, own_name: &str) -> Vec<String> {
    let mut tokens = BTreeSet::new();
    collect_symbol_tokens(node, source, own_name, &mut tokens);
    tokens.into_iter().collect()
}

fn collect_symbol_tokens(
    node: Node<'_>,
    source: &str,
    own_name: &str,
    tokens: &mut BTreeSet<String>,
) {
    let kind = node.kind();
    if is_literal_kind(kind) || (node.named_child_count() == 0 && is_identifier_kind(kind)) {
        let text = child_text(node, source).trim();
        if !text.is_empty() && text != own_name {
            tokens.insert(text.to_owned());
        }
        return;
    }

    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        collect_symbol_tokens(child, source, own_name, tokens);
    }
}

fn is_identifier_kind(kind: &str) -> bool {
    kind == "identifier" || kind.contains("identifier") || kind == "primitive_type"
}

fn is_literal_kind(kind: &str) -> bool {
    kind.contains("literal")
        || matches!(
            kind,
            "string" | "integer" | "float" | "number" | "true" | "false" | "null" | "none" | "nil"
        )
}

#[cfg(test)]
mod gate_contract {
    use std::collections::{BTreeMap, BTreeSet};
    use std::path::Path;

    use tree_sitter::{Parser, Query};

    use super::*;

    #[test]
    fn html_paths_route_to_html_language() {
        for path in ["index.html", "fragment.htm", "UPPER.HTML"] {
            let language = Language::from_path(Path::new(path));
            assert_eq!(language.map(Language::label), Some("html"), "{path}");
        }
        for path in ["theme.scss", "theme.less", "view.vue", "view.svelte"] {
            assert_eq!(Language::from_path(Path::new(path)), None, "{path}");
        }
    }

    #[test]
    fn css_paths_route_to_css_language() {
        for path in ["site.css", "UPPER.CSS"] {
            let language = Language::from_path(Path::new(path));
            assert_eq!(language.map(Language::label), Some("css"), "{path}");
        }
    }

    fn assert_path_query_contract(
        path: &str,
        expected_definitions: &[&str],
        expected_relations: &[&'static str],
    ) {
        let language = Language::from_path(Path::new(path)).expect("language must be registered");
        let config = language.config();
        assert_eq!(
            compiled_definition_captures(language.label(), &config),
            expected_definitions
                .iter()
                .map(|value| (*value).to_owned())
                .collect()
        );
        assert_eq!(
            compiled_relation_predicates(language, language.label(), &config),
            relation_set(expected_relations)
        );
    }

    #[test]
    fn html_query_contract_is_complete() {
        assert_path_query_contract(
            "index.html",
            &["definition.section"],
            &["contains", "defines", "imports", "links"],
        );
    }

    #[test]
    fn css_query_contract_is_complete() {
        assert_path_query_contract(
            "site.css",
            &[
                "definition.constant",
                "definition.function",
                "definition.section",
            ],
            &["contains", "defines", "imports", "links"],
        );
    }

    #[test]
    fn javascript_files_route_to_javascript_language() {
        for path in [
            "a.js", "b.jsx", "c.mjs", "d.cjs", "e.JS", "f.JSX", "g.MJS", "h.CJS",
        ] {
            assert_eq!(
                Language::from_path(Path::new(path)),
                Some(Language::Javascript),
                "{path} should route to JavaScript extraction"
            );
        }

        assert_eq!(
            Language::from_path(Path::new("component.ts")),
            Some(Language::TypeScript),
            ".ts files must continue to route to TypeScript extraction"
        );

        assert_eq!(Language::Javascript.label(), "javascript");
        assert_eq!(
            Language::Javascript.builtin_method_names(),
            TS_BUILTIN_METHODS
        );

        let extensions: BTreeSet<_> = all_supported_extensions().into_iter().collect();
        for extension in ["js", "jsx", "mjs", "cjs"] {
            assert!(
                extensions.contains(extension),
                "supported extensions should include {extension}"
            );
        }
    }

    #[test]
    fn ipynb_path_resolves_to_jupyter_notebook() {
        assert_eq!(
            Language::from_path(Path::new("analysis.ipynb")),
            Some(Language::JupyterNotebook)
        );
    }

    #[test]
    fn tsx_grammar_parses_plain_javascript_with_jsx_without_error() {
        let mut parser = Parser::new();
        let language = tree_sitter_typescript::LANGUAGE_TSX.into();
        parser.set_language(&language).expect("set TSX language");

        let source = r#"
            import { h } from "preact";

            function view(name) {
                return <span>{name}</span>;
            }

            class Widget {
                render() {
                    return view("ok");
                }
            }

            new Widget().render();
        "#;

        let tree = parser.parse(source, None).expect("parse plain JS as TSX");
        assert!(
            !tree.root_node().has_error(),
            "TSX grammar should parse plain JavaScript with JSX without syntax errors"
        );
    }

    #[test]
    fn builtin_method_lists_are_sorted_and_deduplicated() {
        for (label, methods) in [
            ("c", C_BUILTIN_METHODS),
            ("cpp", CPP_BUILTIN_METHODS),
            ("go", GO_BUILTIN_METHODS),
            ("lua", LUA_BUILTIN_METHODS),
            ("python", PYTHON_BUILTIN_METHODS),
            ("rust", RUST_BUILTIN_METHODS),
            ("shell", SHELL_BUILTIN_METHODS),
            ("typescript", TS_BUILTIN_METHODS),
            ("javascript", TS_BUILTIN_METHODS),
            (
                "jupyter_notebook",
                Language::JupyterNotebook.builtin_method_names(),
            ),
        ] {
            assert!(
                methods.is_sorted(),
                "{label}: builtin methods must be sorted"
            );
            assert!(
                methods.windows(2).all(|window| window[0] != window[1]),
                "{label}: builtin methods must be de-duplicated"
            );
        }
    }

    #[test]
    fn every_registered_language_satisfies_query_contract() {
        assert_registry_extensions_are_unique();

        for descriptor in language_registry() {
            let language = descriptor.language;
            let config = (descriptor.factory)();
            let label = language.label();

            if is_container_language(language) {
                assert_eq!(
                    descriptor.extensions,
                    &["ipynb"],
                    "{label}: container language extension contract changed"
                );
                assert!(
                    config.queries.is_empty(),
                    "{label}: container extraction must not declare tree-sitter queries"
                );
                assert!(
                    config.definition_kind_map.is_empty(),
                    "{label}: container extraction must not declare direct definitions"
                );
                continue;
            }

            let tags_source = config
                .queries
                .iter()
                .find_map(|(name, source)| (*name == "tags").then_some(*source))
                .unwrap_or_else(|| panic!("{label}: missing `tags` query"));
            assert!(
                !tags_source.trim().is_empty(),
                "{label}: `tags` query source must not be empty"
            );

            let definition_captures = compiled_definition_captures(label, &config);
            let mapped_definitions: BTreeSet<_> = config
                .definition_kind_map
                .iter()
                .map(|(capture_name, _)| (*capture_name).to_owned())
                .collect();

            assert!(
                !mapped_definitions.is_empty(),
                "{label}: definition_kind_map must not be empty"
            );

            let orphan_captures: BTreeSet<_> = definition_captures
                .difference(&mapped_definitions)
                .cloned()
                .collect();
            assert!(
                orphan_captures.is_empty(),
                "{label}: @definition.* captures missing from definition_kind_map: {orphan_captures:?}"
            );

            let unused_mappings: BTreeSet<_> = mapped_definitions
                .difference(&definition_captures)
                .cloned()
                .collect();
            assert!(
                unused_mappings.is_empty(),
                "{label}: definition_kind_map entries missing from compiled queries: {unused_mappings:?}"
            );

            for (capture_name, kind) in config.definition_kind_map {
                assert_ne!(
                    symbol_kind(*kind),
                    "symbol",
                    "{label}: `{capture_name}` maps to {kind:?}, but symbol_kind() falls back to `symbol`"
                );
            }

            let expected_definitions: BTreeSet<_> = expected_definition_captures(language)
                .iter()
                .map(|capture_name| (*capture_name).to_owned())
                .collect();
            let missing_expected: BTreeSet<_> = expected_definitions
                .difference(&definition_captures)
                .cloned()
                .collect();
            assert!(
                missing_expected.is_empty(),
                "{label}: expected definition captures missing from compiled queries: {missing_expected:?}"
            );
        }
    }

    #[test]
    fn relation_coverage_matches_declared_contract() {
        let expected = expected_relation_predicates();
        let mut actual = BTreeMap::new();

        for descriptor in language_registry() {
            let language = descriptor.language;
            let config = (descriptor.factory)();
            let label = language.label();
            let expected_relations = expected
                .get(label)
                .unwrap_or_else(|| panic!("{label}: missing relation coverage contract row"));
            let realized = compiled_relation_predicates(language, label, &config);

            for gap in allowlisted_relation_gaps(language) {
                assert!(
                    !expected_relations.contains(gap),
                    "{label}: README TODO gap `{gap}` must be absent from the expected relation set"
                );
                assert!(
                    !realized.contains(gap),
                    "{label}: README TODO gap `{gap}` is now realized; update the README matrix and expected table together"
                );
            }

            actual.insert(label, realized);
        }

        assert_eq!(
            actual, expected,
            "relation coverage matrix drifted; update the README matrix and expected table together, or emit risk if the mismatch is unexpected"
        );
    }

    fn assert_registry_extensions_are_unique() {
        let mut owners: BTreeMap<&str, Language> = BTreeMap::new();
        for descriptor in language_registry() {
            assert!(
                !descriptor.extensions.is_empty(),
                "{}: registry entry must declare at least one extension",
                descriptor.label
            );
            for extension in descriptor.extensions {
                assert!(
                    !extension.is_empty(),
                    "{}: registry extension must not be empty",
                    descriptor.label
                );
                if let Some(previous) = owners.insert(*extension, descriptor.language) {
                    assert_eq!(
                        previous,
                        descriptor.language,
                        "extension `{extension}` is registered for both {} and {}",
                        previous.label(),
                        descriptor.language.label()
                    );
                }
            }
        }
    }

    fn compiled_definition_captures(
        language_label: &str,
        config: &LanguageConfig,
    ) -> BTreeSet<String> {
        let mut captures = BTreeSet::new();
        for (name, source) in config.queries {
            let query_language = query_language_for(config, name, language_label);
            let query = Query::new(query_language, source).unwrap_or_else(|err| {
                panic!("failed to compile tree-sitter query `{name}` for `{language_label}`: {err}")
            });
            captures.extend(
                query
                    .capture_names()
                    .iter()
                    .filter(|capture_name| capture_name.starts_with("definition."))
                    .map(|capture_name| (*capture_name).to_owned()),
            );
        }
        captures
    }

    fn compiled_relation_predicates(
        language: Language,
        language_label: &str,
        config: &LanguageConfig,
    ) -> BTreeSet<&'static str> {
        let mut relations = BTreeSet::new();

        for capture_name in compiled_capture_names(language_label, config) {
            if let Some(relation) = relation_kind_for_edge_capture(config, &capture_name) {
                relations.insert(relation_predicate(relation));
            }
        }

        let definition_captures = compiled_definition_captures(language_label, config);
        if is_container_language(language) {
            relations.insert(relation_predicate(RelationKind::Contains));
            relations.insert(relation_predicate(RelationKind::Produces));
            relations.insert(relation_predicate(RelationKind::Consumes));
            relations.insert(relation_predicate(RelationKind::Binds));
            relations.insert(relation_predicate(RelationKind::Emits));
            relations.insert(relation_predicate(RelationKind::References));
            return relations;
        }
        if !definition_captures.is_empty() {
            relations.insert(relation_predicate(RelationKind::Contains));
            if language != Language::Markdown {
                relations.insert(relation_predicate(RelationKind::Defines));
            }
        }

        if relations.contains(relation_predicate(RelationKind::Calls))
            && has_construct_targets(config)
        {
            relations.insert(relation_predicate(RelationKind::Constructs));
        }

        if language == Language::Python
            && relations.contains(relation_predicate(RelationKind::Extends))
        {
            // Python `extends` captures targeting Protocol/ABC classes are
            // reclassified to Implements after resolution.
            relations.insert(relation_predicate(RelationKind::Implements));
        }

        relations
    }

    fn compiled_capture_names(language_label: &str, config: &LanguageConfig) -> BTreeSet<String> {
        let mut captures = BTreeSet::new();
        for (name, source) in config.queries {
            let query_language = query_language_for(config, name, language_label);
            let query = Query::new(query_language, source).unwrap_or_else(|err| {
                panic!("failed to compile tree-sitter query `{name}` for `{language_label}`: {err}")
            });
            captures.extend(
                query
                    .capture_names()
                    .iter()
                    .map(|capture_name| (*capture_name).to_owned()),
            );
        }
        captures
    }

    fn relation_kind_for_edge_capture(
        config: &LanguageConfig,
        capture_name: &str,
    ) -> Option<RelationKind> {
        // Keep this dispatch in lockstep with emit_edges plus Markdown link
        // emitters. Helper captures such as `import.name` and `call.name` do
        // not create edges by themselves.
        match capture_name {
            "import" => Some(relation_kind_for_capture(
                config,
                "import",
                RelationKind::Imports,
            )),
            "reexport" => Some(relation_kind_for_capture(
                config,
                "reexport",
                RelationKind::Imports,
            )),
            "link" => Some(RelationKind::Links),
            "call" | "jsx_call" | "macro_call" => Some(RelationKind::Calls),
            "implements" => Some(RelationKind::Implements),
            "extends" => Some(RelationKind::Extends),
            "reference.name" => Some(RelationKind::References),
            "hcl_ref" => Some(RelationKind::References),
            _ => None,
        }
    }

    fn has_construct_targets(config: &LanguageConfig) -> bool {
        config.definition_kind_map.iter().any(|(_, kind)| {
            matches!(
                kind,
                NodeKind::Class | NodeKind::Struct | NodeKind::EnumVariant
            )
        })
    }

    fn relation_predicate(relation: RelationKind) -> &'static str {
        match relation {
            RelationKind::Imports => "imports",
            RelationKind::Calls => "calls",
            RelationKind::Constructs => "constructs",
            RelationKind::Contains => "contains",
            RelationKind::Implements => "implements",
            RelationKind::Defines => "defines",
            RelationKind::References => "references",
            RelationKind::Extends => "extends",
            RelationKind::Links => "links",
            RelationKind::Touches => "touches",
            RelationKind::Produces => "produces",
            RelationKind::Consumes => "consumes",
            RelationKind::Binds => "binds",
            RelationKind::Emits => "emits",
        }
    }

    fn expected_relation_predicates() -> BTreeMap<&'static str, BTreeSet<&'static str>> {
        BTreeMap::from([
            (
                Language::Rust.label(),
                relation_set(&[
                    "imports",
                    "calls",
                    "constructs",
                    "contains",
                    "defines",
                    "references",
                    "implements",
                    "extends",
                ]),
            ),
            (
                Language::Python.label(),
                relation_set(&[
                    "imports",
                    "calls",
                    "constructs",
                    "contains",
                    "defines",
                    "references",
                    "implements",
                    "extends",
                ]),
            ),
            (
                Language::TypeScript.label(),
                relation_set(&[
                    "imports",
                    "calls",
                    "constructs",
                    "contains",
                    "defines",
                    "references",
                    "implements",
                    "extends",
                ]),
            ),
            (
                Language::Tsx.label(),
                relation_set(&[
                    "imports",
                    "calls",
                    "constructs",
                    "contains",
                    "defines",
                    "references",
                    "implements",
                    "extends",
                ]),
            ),
            (
                Language::Javascript.label(),
                relation_set(&[
                    "imports",
                    "calls",
                    "constructs",
                    "contains",
                    "defines",
                    "references",
                    "implements",
                    "extends",
                ]),
            ),
            (
                Language::C.label(),
                relation_set(&["imports", "calls", "constructs", "contains", "defines"]),
            ),
            (
                Language::Cpp.label(),
                relation_set(&[
                    "imports",
                    "calls",
                    "constructs",
                    "contains",
                    "defines",
                    "references",
                    "extends",
                ]),
            ),
            (
                Language::Go.label(),
                relation_set(&["imports", "calls", "constructs", "contains", "defines"]),
            ),
            (
                Language::Hcl.label(),
                relation_set(&["contains", "defines", "references"]),
            ),
            (
                Language::Terraform.label(),
                relation_set(&["contains", "defines", "references"]),
            ),
            (
                Language::Lua.label(),
                relation_set(&["imports", "calls", "contains", "defines"]),
            ),
            (
                Language::Shell.label(),
                relation_set(&["imports", "calls", "contains", "defines"]),
            ),
            (
                Language::Sql.label(),
                relation_set(&["calls", "constructs", "contains", "defines", "references"]),
            ),
            (
                Language::Json.label(),
                relation_set(&["contains", "defines"]),
            ),
            (
                Language::Toml.label(),
                relation_set(&["contains", "defines"]),
            ),
            (
                Language::Yaml.label(),
                relation_set(&["contains", "defines"]),
            ),
            (
                Language::Mermaid.label(),
                relation_set(&["contains", "defines"]),
            ),
            (
                Language::Html.label(),
                relation_set(&["imports", "links", "contains", "defines"]),
            ),
            (
                Language::Css.label(),
                relation_set(&["imports", "links", "contains", "defines"]),
            ),
            (
                Language::Markdown.label(),
                // README's Markdown `imports` cell is `Y(links)`: the
                // @import capture channel is remapped to RelationKind::Links.
                relation_set(&["contains", "links"]),
            ),
            (
                Language::JupyterNotebook.label(),
                relation_set(&[
                    "contains",
                    "produces",
                    "consumes",
                    "binds",
                    "emits",
                    "references",
                ]),
            ),
        ])
    }

    fn relation_set(predicates: &[&'static str]) -> BTreeSet<&'static str> {
        predicates.iter().copied().collect()
    }

    fn allowlisted_relation_gaps(_language: Language) -> &'static [&'static str] {
        &[]
    }

    fn query_language_for<'a>(
        config: &'a LanguageConfig,
        query_name: &str,
        language_label: &str,
    ) -> &'a TsLanguage {
        match query_name {
            "inline-spur-edges" => config.inline_language.as_ref().unwrap_or_else(|| {
                panic!("{language_label}: query `inline-spur-edges` requires an inline language")
            }),
            _ => &config.language,
        }
    }

    fn expected_definition_captures(language: Language) -> &'static [&'static str] {
        match language {
            Language::Rust => &[
                "definition.module",
                "definition.function",
                "definition.method",
                "definition.struct",
                "definition.enum",
                "definition.enum_variant",
                "definition.field",
                "definition.trait",
                "definition.impl",
                "definition.type_alias",
                "definition.constant",
                "definition.macro",
            ],
            Language::Python => &[
                "definition.function",
                "definition.class",
                "definition.constant",
            ],
            Language::TypeScript | Language::Tsx | Language::Javascript => &[
                "definition.class",
                "definition.interface",
                "definition.enum",
                "definition.enum_variant",
                "definition.function",
                "definition.method",
                "definition.field",
                "definition.type_alias",
                "definition.constant",
                "definition.module",
            ],
            Language::C => &[
                "definition.struct",
                "definition.enum",
                "definition.enum_variant",
                "definition.function",
                "definition.type_alias",
                "definition.macro",
                "definition.field",
                "definition.constant",
            ],
            Language::Cpp => &[
                "definition.module",
                "definition.class",
                "definition.struct",
                "definition.enum",
                "definition.enum_variant",
                "definition.function",
                "definition.method",
                "definition.type_alias",
                "definition.macro",
                "definition.field",
                "definition.constant",
            ],
            Language::Go => &[
                "definition.module",
                "definition.function",
                "definition.method",
                "definition.struct",
                "definition.interface",
                "definition.type_alias",
                "definition.constant",
                "definition.field",
            ],
            Language::Hcl | Language::Terraform => &[
                "definition.resource",
                "definition.data",
                "definition.module",
                "definition.variable",
                "definition.output",
                "definition.local",
            ],
            Language::Lua => &["definition.function", "definition.method"],
            Language::Shell => &["definition.function", "definition.constant"],
            Language::Sql => &[
                "definition.struct",
                "definition.type_alias",
                "definition.function",
                "definition.enum",
                "definition.field",
                "definition.module",
                "definition.constant",
            ],
            Language::Markdown => &["definition.section"],
            Language::JupyterNotebook => &[],
            Language::Json | Language::Toml | Language::Yaml => &[
                "definition.module",
                "definition.field",
                "definition.constant",
            ],
            Language::Mermaid => &["definition.module"],
            Language::Html => &["definition.section"],
            Language::Css => &[
                "definition.constant",
                "definition.function",
                "definition.section",
            ],
        }
    }

    fn is_container_language(language: Language) -> bool {
        matches!(language, Language::JupyterNotebook)
    }
}

fn contained_capture_text(
    parent: &CaptureHit<'_>,
    source: &str,
    captures: &[CaptureHit<'_>],
    capture_name: &str,
) -> Vec<String> {
    captures
        .iter()
        .filter(|capture| {
            capture.name == capture_name
                && capture.pattern_index == parent.pattern_index
                && capture.match_index == parent.match_index
        })
        .map(|capture| child_text(capture.node, source).trim().to_owned())
        .filter(|text| !text.is_empty())
        .collect()
}

fn import_path_for_edge(import_paths: &[String], preserve_bare: bool) -> Option<String> {
    import_paths
        .iter()
        .filter_map(|path| normalize_import_path(path, preserve_bare))
        .next()
}

fn normalize_import_path(path: &str, preserve_bare: bool) -> Option<String> {
    let path = path.trim();
    if path.is_empty() || (!preserve_bare && is_bare_single_segment_import(path)) {
        return None;
    }
    Some(path.to_owned())
}

fn is_bare_single_segment_import(path: &str) -> bool {
    !path.starts_with('.')
        && !path.contains("::")
        && !path.contains('.')
        && !path.contains('/')
        && !path.contains('\\')
        && !path.contains('{')
}

fn child_text<'a>(node: Node<'_>, source: &'a str) -> &'a str {
    &source[node.start_byte()..node.end_byte()]
}

fn string_lit_text(node: Node<'_>, source: &str) -> Option<String> {
    if node.kind() != "string_lit" {
        return None;
    }
    let mut literal = None;
    for index in 0..node.named_child_count() {
        let child = node.named_child(index as u32)?;
        match child.kind() {
            "template_literal" if literal.is_none() => literal = Some(child),
            "quoted_template_start" | "quoted_template_end" => {}
            _ => return None,
        }
    }
    literal.map(|literal| child_text(literal, source).to_owned())
}

fn scoped_name(prefix: &str, name: &str) -> String {
    if prefix.is_empty() {
        name.to_owned()
    } else {
        format!("{prefix}::{name}")
    }
}
