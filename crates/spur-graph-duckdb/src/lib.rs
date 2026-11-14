//! Native DuckDB 2.0.0 / C API v2 adapter.
//!
//! `spur_graph_symbols(logical_path, source)` returns LIST<STRUCT> facts using
//! the existing SPUR language registry and extractor. Callers retain blob/path
//! lineage in ordinary columns and expand facts with SQL UNNEST. An empty list
//! is a completed extraction with no symbols; a NULL argument produces NULL.
//!
//! Parser/query state is local to a query worker and reused across input
//! chunks. The function performs no filesystem, Git, embedding or history work.

use std::collections::HashMap;
use std::path::Path;

use duckdb_neo::connection::Context;
use duckdb_neo::data_chunk::VectorCollection;
use duckdb_neo::error::Error;
use duckdb_neo::extension::Extension;
use duckdb_neo::scalar::{ScalarCallbacks, ScalarFunctionBuilder, ScalarInitCallbacks};
use duckdb_neo::signature::{FunctionDocumentationBuilder, FunctionParameter, SignatureBuilder};
use duckdb_neo::types::List;
use duckdb_neo::vector::{RowWriter, Unknown, Vector};
use duckdb_neo::{duckdb_entrypoint_c_api_v2, AsDuckDB, DuckDBType, Result};
use spur_graph::extract::languages::Language;
use spur_graph::extract::tree_sitter::{BytesExtractor, ExtractedSymbol};

#[derive(AsDuckDB)]
#[duckdb(writer)]
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
    .register(extension)
}
