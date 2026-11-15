//! Native DuckDB 2.0.0 / C API v2 adapter.
//!
//! `spur_graph_symbols(logical_path, source)` returns LIST<STRUCT> facts using
//! the existing SPUR language registry and extractor. Callers retain blob/path
//! lineage in ordinary columns and expand facts with SQL UNNEST. An empty list
//! is a completed extraction with no symbols; a NULL argument produces NULL.
//!
//! Parser/query state is local to a query worker and reused across input
//! chunks. The function performs no filesystem, Git, embedding or history work.

mod file_facts;

use std::collections::HashMap;
use std::path::Path;

use duckdb_neo::connection::Context;
use duckdb_neo::data_chunk::VectorCollection;
use duckdb_neo::error::Error;
use duckdb_neo::extension::Extension;
use duckdb_neo::scalar::{ScalarCallbacks, ScalarFunctionBuilder, ScalarInitCallbacks};
use duckdb_neo::signature::{FunctionDocumentationBuilder, FunctionParameter, SignatureBuilder};
use duckdb_neo::types::List;
use duckdb_neo::vector::{Elements, RowReader, RowWriter, Unknown, Vector};
use duckdb_neo::{duckdb_entrypoint_c_api_v2, AsDuckDB, DuckDBType, Result};
use spur_graph::extract::languages::Language;
use spur_graph::extract::tree_sitter::{BytesExtractor, ExtractedSymbol};
use spur_graph::git_walk::{
    symbol_changes_from_extracted, FileChange, FileChangeKind, SymbolChange,
};
use spur_graph::schema::{ChangeKind, GitPath, RenamePrev};

#[derive(AsDuckDB)]
#[duckdb(reader, writer)]
struct SymbolFact {
    entity_name: String,
    symbol_kind: String,
    enclosing_scope: Option<String>,
    byte_start: u64,
    byte_end: u64,
    line_start: u64,
    line_end: u64,
    anchor_hash: String,
    tokens: List<String>,
}

#[derive(AsDuckDB)]
#[duckdb(writer)]
struct SymbolDelta {
    symbol: SymbolFact,
    stable_symbol_id: String,
    file_path: String,
    change_kind: String,
    previous_stable_symbol_id: Option<String>,
}

#[derive(AsDuckDB)]
#[duckdb(writer)]
struct SymbolDiff {
    changes: List<SymbolDelta>,
    diagnostics: List<String>,
}

impl From<SymbolChange> for SymbolDelta {
    fn from(change: SymbolChange) -> Self {
        let (change_kind, previous_stable_symbol_id) = match change.change_kind {
            ChangeKind::Added => ("added", None),
            ChangeKind::Modified => ("modified", None),
            ChangeKind::Deleted => ("deleted", None),
            ChangeKind::RenamedFrom(RenamePrev::Symbol(previous)) => {
                ("renamed_from_symbol", Some(previous.stable_symbol_id))
            }
            // The shared pair matcher emits only symbol predecessors.
            ChangeKind::RenamedFrom(RenamePrev::File(_)) => {
                unreachable!("pair matcher file predecessor")
            }
        };
        let snapshot = change.snapshot;
        Self {
            symbol: ExtractedSymbol {
                entity_name: snapshot.entity_name,
                symbol_kind: snapshot.symbol_kind,
                enclosing_scope: snapshot.enclosing_scope,
                byte_range: snapshot.byte_range,
                line_range: snapshot.line_range,
                anchor_hash: snapshot.anchor_hash,
                tokens: snapshot.tokens,
            }
            .into(),
            stable_symbol_id: snapshot.key.stable_symbol_id,
            file_path: snapshot.file_path.to_string_lossy().into_owned(),
            change_kind: change_kind.to_owned(),
            previous_stable_symbol_id,
        }
    }
}

fn required<T>(value: Option<T>, field: &str) -> Result<T> {
    value.ok_or_else(|| {
        Error::invalid_input(format!(
            "spur_graph_symbol_changes: NULL required field {field}"
        ))
    })
}

fn source_offset(value: Option<&u64>, field: &str) -> Result<usize> {
    usize::try_from(*required(value, field)?).map_err(|_| {
        Error::invalid_input(format!(
            "spur_graph_symbol_changes: {field} exceeds host usize"
        ))
    })
}

fn read_symbols(symbols: Elements<'_, SymbolFactReader<'_>>) -> Result<Vec<ExtractedSymbol>> {
    symbols
        .iter()
        .map(|symbol| {
            let symbol = required(symbol, "symbol list element")?;
            Ok(ExtractedSymbol {
                entity_name: required(symbol.entity_name, "entity_name")?.to_owned(),
                symbol_kind: required(symbol.symbol_kind, "symbol_kind")?.to_owned(),
                enclosing_scope: symbol.enclosing_scope.map(str::to_owned),
                byte_range: [
                    source_offset(symbol.byte_start, "byte_start")?,
                    source_offset(symbol.byte_end, "byte_end")?,
                ],
                line_range: [
                    source_offset(symbol.line_start, "line_start")?,
                    source_offset(symbol.line_end, "line_end")?,
                ],
                anchor_hash: required(symbol.anchor_hash, "anchor_hash")?.to_owned(),
                tokens: required(symbol.tokens, "tokens")?
                    .iter()
                    .map(|token| required(token, "token list element").map(str::to_owned))
                    .collect::<Result<_>>()?,
            })
        })
        .collect()
}

struct MatchSymbols;

impl ScalarCallbacks for MatchSymbols {
    type BindData = ();
    type InitData = ();

    fn exec(
        &self,
        _context: &Context,
        _bind_data: Option<&Self::BindData>,
        _init_data: Option<&mut Self::InitData>,
        vectors: &mut VectorCollection,
        output: Vector<'_, Unknown>,
    ) -> Result<()> {
        let paths = vectors.get_vector_at::<String>(0)?;
        let old_paths = vectors.get_vector_at::<String>(1)?;
        let kinds = vectors.get_vector_at::<String>(2)?;
        let old_symbols = vectors.get_vector_at::<List<SymbolFact>>(3)?;
        let new_symbols = vectors.get_vector_at::<List<SymbolFact>>(4)?;
        let paths = paths.reader()?;
        let old_paths = old_paths.reader()?;
        let kinds = kinds.reader()?;
        let old_symbols = old_symbols.list_reader()?;
        let new_symbols = new_symbols.list_reader()?;
        let mut rows = Vec::with_capacity(vectors.row_count()?);
        for index in 0..vectors.row_count()? {
            let (Some(path), Some(old_path), Some(kind), Some(old), Some(new)) = (
                paths.get(index)?,
                old_paths.get(index)?,
                kinds.get(index)?,
                old_symbols.get(index)?,
                new_symbols.get(index)?,
            ) else {
                rows.push(None);
                continue;
            };
            let kind = match kind {
                "A" => FileChangeKind::Added,
                "M" => FileChangeKind::Modified,
                "D" => FileChangeKind::Deleted,
                "R" => FileChangeKind::Renamed {
                    from: GitPath::from(old_path),
                },
                _ => {
                    return Err(Error::invalid_input(
                        "spur_graph_symbol_changes: kind must be A, M, D, or R; T and C are unsupported",
                    ));
                }
            };
            if !matches!(kind, FileChangeKind::Renamed { .. })
                && old_path != path
                && !(matches!(kind, FileChangeKind::Added) && old_path.is_empty())
            {
                return Err(Error::invalid_input(
                    "spur_graph_symbol_changes: old_path must equal path for non-renames (or be empty for A)",
                ));
            }
            let file_change = FileChange {
                path: GitPath::from(path),
                kind,
                parent_sha: None,
            };
            let (changes, diagnostics) = symbol_changes_from_extracted(
                "__pair__",
                &file_change,
                &read_symbols(old)?,
                &read_symbols(new)?,
            );
            rows.push(Some(SymbolDiff {
                changes: changes
                    .into_iter()
                    .map(|change| Some(change.into()))
                    .collect(),
                diagnostics: diagnostics.into_iter().map(Some).collect(),
            }));
        }

        let change_count = rows.iter().flatten().map(|row| row.changes.0.len()).sum();
        let token_count = rows
            .iter()
            .flatten()
            .flat_map(|row| row.changes.0.iter().flatten())
            .map(|change| change.symbol.tokens.0.len())
            .sum();
        let diagnostic_count = rows
            .iter()
            .flatten()
            .map(|row| row.diagnostics.0.len())
            .sum();
        let mut output = output.cast::<SymbolDiff>()?;
        output.set_size(rows.len())?;
        {
            let mut fields = output.children_mut().ok_or_else(|| {
                Error::internal("spur_graph_symbol_changes: result fields missing")
            })?;
            {
                let mut changes = fields.get_vector_at_mut::<List<SymbolDelta>>(0)?;
                changes.set_child_size(change_count)?;
                let mut elements = changes.children_mut().ok_or_else(|| {
                    Error::internal("spur_graph_symbol_changes: change elements missing")
                })?;
                let mut changes = elements.get_vector_at_mut::<SymbolDelta>(0)?;
                let mut fields = changes.children_mut().ok_or_else(|| {
                    Error::internal("spur_graph_symbol_changes: change fields missing")
                })?;
                let mut symbols = fields.get_vector_at_mut::<SymbolFact>(0)?;
                symbols
                    .children_mut()
                    .ok_or_else(|| {
                        Error::internal("spur_graph_symbol_changes: symbol fields missing")
                    })?
                    .get_vector_at_mut::<List<String>>(8)?
                    .set_child_size(token_count)?;
            }
            fields
                .get_vector_at_mut::<List<String>>(1)?
                .set_child_size(diagnostic_count)?;
        }
        // Every LIST descendant is allocated before the writer borrows buffers.
        let mut writer = output.row_writer()?;
        for (index, row) in rows.iter().enumerate() {
            writer.set_option(index, row.as_ref())?;
        }
        Ok(())
    }
}

impl From<ExtractedSymbol> for SymbolFact {
    fn from(symbol: ExtractedSymbol) -> Self {
        Self {
            entity_name: symbol.entity_name,
            symbol_kind: symbol.symbol_kind,
            enclosing_scope: symbol.enclosing_scope,
            byte_start: symbol.byte_range[0] as u64,
            byte_end: symbol.byte_range[1] as u64,
            line_start: symbol.line_range[0] as u64,
            line_end: symbol.line_range[1] as u64,
            anchor_hash: symbol.anchor_hash,
            tokens: symbol.tokens.into_iter().map(Some).collect(),
        }
    }
}

struct ParseSymbols;

impl ScalarInitCallbacks for ParseSymbols {
    fn init(
        &self,
        _context: &Context,
        _bind_data: Option<&Self::BindData>,
    ) -> Result<Self::InitData> {
        Ok(HashMap::new())
    }
}

impl ScalarCallbacks for ParseSymbols {
    type BindData = ();
    type InitData = HashMap<Language, BytesExtractor>;

    fn exec(
        &self,
        _context: &Context,
        _bind_data: Option<&Self::BindData>,
        init_data: Option<&mut Self::InitData>,
        vectors: &mut VectorCollection,
        output: Vector<'_, Unknown>,
    ) -> Result<()> {
        let parsers =
            init_data.ok_or_else(|| Error::internal("spur_graph_symbols: missing worker state"))?;
        let paths = vectors.get_vector_at::<String>(0)?;
        let sources = vectors.get_vector_at::<String>(1)?;
        let path_reader = paths.reader()?;
        let source_reader = sources.reader()?;
        let mut rows: Vec<Option<List<SymbolFact>>> = Vec::with_capacity(vectors.row_count()?);

        for (path, source) in path_reader.iter().zip(source_reader.iter()) {
            let (Some(path), Some(source)) = (path, source) else {
                rows.push(None);
                continue;
            };
            let path = Path::new(path);
            let language = Language::from_path(path).ok_or_else(|| {
                Error::invalid_input(format!("unsupported language for path: {}", path.display()))
            })?;
            if let std::collections::hash_map::Entry::Vacant(entry) = parsers.entry(language) {
                entry.insert(BytesExtractor::for_language(language).map_err(|error| {
                    Error::invalid_input(format!("spur_graph_symbols: {error}"))
                })?);
            }
            let parser = parsers
                .get_mut(&language)
                .ok_or_else(|| Error::internal("spur_graph_symbols: parser state missing"))?;
            let symbols = parser
                .extract(path, source.as_bytes())
                .map_err(|error| Error::invalid_input(format!("spur_graph_symbols: {error}")))?;
            rows.push(Some(
                symbols
                    .into_iter()
                    .map(|symbol| Some(symbol.into()))
                    .collect(),
            ));
        }

        let symbol_count = rows.iter().flatten().map(|row| row.0.len()).sum();
        let token_count = rows
            .iter()
            .flatten()
            .flat_map(|row| row.0.iter().flatten())
            .map(|symbol| symbol.tokens.0.len())
            .sum();
        let mut output = output.cast::<List<SymbolFact>>()?;
        output.set_size(rows.len())?;
        output.set_child_size(symbol_count)?;
        {
            let mut children = output
                .children_mut()
                .ok_or_else(|| Error::internal("spur_graph_symbols: list child missing"))?;
            let mut symbols = children.get_vector_at_mut::<SymbolFact>(0)?;
            // The typed struct schema above places tokens after the eight scalar fields.
            // The typed lookup checks the nested field before it is resized.
            symbols
                .children_mut()
                .ok_or_else(|| Error::internal("spur_graph_symbols: struct children missing"))?
                .get_vector_at_mut::<List<String>>(8)?
                .set_child_size(token_count)?;
        }
        let mut writer = output.list_writer()?;
        for (index, row) in rows.iter().enumerate() {
            match row {
                Some(symbols) => writer.set(index, symbols)?,
                None => writer.set_null(index)?,
            }
        }
        Ok(())
    }
}

#[duckdb_entrypoint_c_api_v2(extension_name = "spur_graph", api_version = "2.0.0")]
pub fn entrypoint(extension: &Extension) -> Result<()> {
    file_facts::register(extension)?;
    ScalarFunctionBuilder::new(
        "spur_graph_symbols",
        SignatureBuilder::new()
            .param(FunctionParameter::new::<String>("logical_path"))
            .param(FunctionParameter::new::<String>("source"))
            .return_type(List::<SymbolFact>::logical_type(extension)?)
            .documentation(
                FunctionDocumentationBuilder::new()
                    .description("Extract SPUR symbol facts from source text using the language of its logical path")
                    .add_example("Parse Rust source", "SELECT unnest(spur_graph_symbols('input.rs', 'pub fn alpha() {}'), recursive := true)")
                    .add_category("spur_graph"),
            ),
        ParseSymbols,
    )
    .with_init()
    .register(extension)?;

    ScalarFunctionBuilder::new(
        "spur_graph_symbol_changes",
        SignatureBuilder::new()
            .param(FunctionParameter::new::<String>("path"))
            .param(FunctionParameter::new::<String>("old_path"))
            .param(FunctionParameter::new::<String>("kind"))
            .param(FunctionParameter::new::<List<SymbolFact>>("old_symbols"))
            .param(FunctionParameter::new::<List<SymbolFact>>("new_symbols"))
            .return_type(SymbolDiff::logical_type(extension)?)
            .documentation(
                FunctionDocumentationBuilder::new()
                    .description("Match ordered extracted symbol facts for one file-change pair; project commit and parent lineage separately")
                    .add_category("spur_graph"),
            ),
        MatchSymbols,
    )
    .register(extension)
}
