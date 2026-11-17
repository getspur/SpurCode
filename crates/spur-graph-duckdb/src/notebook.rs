//! Additive notebook contract. Core owns normalization, preparation and matching;
//! this module only translates typed facts and sizes DuckDB's nested vectors.

use duckdb_neo::connection::Context;
use duckdb_neo::data_chunk::{ChildrenMut, VectorCollection};
use duckdb_neo::enums::FunctionProperty;
use duckdb_neo::error::Error;
use duckdb_neo::extension::Extension;
use duckdb_neo::scalar::{ScalarCallbacks, ScalarFunctionBuilder};
use duckdb_neo::signature::{FunctionDocumentationBuilder, FunctionParameter, SignatureBuilder};
use duckdb_neo::types::List;
use duckdb_neo::vector::{NativeElement, RowWriter, Unknown, Vector};
use duckdb_neo::{AsDuckDB, DuckDBType, Result};
use spur_graph::extract::notebook_model as model;
use spur_graph::extract::notebook_symbols as core;

use super::SymbolFact;

#[derive(AsDuckDB)]
#[duckdb(writer)]
struct Diagnostic {
    code: String,
    side: Option<String>,
    cell_ordinal: Option<u64>,
    byte_start: Option<u64>,
    byte_end: Option<u64>,
    message: String,
}

#[derive(AsDuckDB)]
#[duckdb(writer)]
struct SourceRef {
    notebook_path: String,
    notebook_source_sha256: String,
    cell_id: String,
    cell_ordinal: u64,
    cell_identity_quality: String,
    cell_source_sha256: String,
    cell_language: String,
    coordinate_space: String,
    byte_start: u64,
    byte_end: u64,
    line_start: u64,
    line_end: u64,
}

#[derive(AsDuckDB)]
#[duckdb(writer)]
struct Symbol {
    id: String,
    symbol: SymbolFact,
    source_ref: SourceRef,
}

#[derive(AsDuckDB)]
#[duckdb(writer)]
struct Directive {
    text: String,
    byte_start: u64,
    byte_end: u64,
}

#[derive(AsDuckDB)]
#[duckdb(writer)]
struct PreparedSource {
    original: String,
    original_sha256: String,
    analysis: String,
    analysis_sha256: String,
    policy: String,
    coordinate_map: String,
    directives: List<Directive>,
}

#[derive(AsDuckDB)]
#[duckdb(writer)]
struct Cell {
    id: String,
    identity_quality: String,
    ordinal: u64,
    cell_type: Option<String>,
    language: Option<String>,
    metadata_json: String,
    source: Option<PreparedSource>,
    status: String,
    diagnostics: List<Diagnostic>,
    symbols: List<Symbol>,
}

#[derive(AsDuckDB)]
#[duckdb(writer)]
struct Document {
    contract_version: u32,
    path: String,
    source_sha256: String,
    metadata_json: String,
    cells: List<Cell>,
    status: String,
    diagnostics: List<Diagnostic>,
}

#[derive(AsDuckDB)]
#[duckdb(writer)]
struct SymbolChange {
    kind: String,
    before: Option<Symbol>,
    after: Option<Symbol>,
}

#[derive(AsDuckDB)]
#[duckdb(writer)]
struct CellRef {
    notebook_path: String,
    notebook_source_sha256: String,
    cell_id: String,
    cell_ordinal: u64,
    cell_source_sha256: Option<String>,
    language: Option<String>,
    cell_type: Option<String>,
}

#[derive(AsDuckDB)]
#[duckdb(writer)]
struct CellChange {
    kind: String,
    before: Option<CellRef>,
    after: Option<CellRef>,
}

#[derive(AsDuckDB)]
#[duckdb(writer)]
struct DocumentChange {
    kind: String,
    before_path: Option<String>,
    after_path: Option<String>,
    before_metadata_json: Option<String>,
    after_metadata_json: Option<String>,
}

#[derive(AsDuckDB)]
#[duckdb(writer)]
struct Comparison {
    contract_version: u32,
    status: String,
    diagnostics: List<Diagnostic>,
    before: Option<Document>,
    after: Option<Document>,
    symbol_changes: List<SymbolChange>,
    cell_changes: List<CellChange>,
    document_changes: List<DocumentChange>,
}

// Exhaustive mappings freeze SQL spellings independently of Debug formatting.
macro_rules! enum_name {
    ($fn:ident, $type:path, {$($variant:ident => $name:literal),+ $(,)?}) => {
        fn $fn(value: $type) -> &'static str {
            use $type as E;
            match value { $(E::$variant => $name),+ }
        }
    };
}
enum_name!(status, model::NotebookStatus, {
    Complete => "complete", Partial => "partial", Invalid => "invalid"
});
enum_name!(identity, model::CellIdentityQuality, {
    Explicit => "explicit", SourceDerived => "source_derived", Ambiguous => "ambiguous"
});
enum_name!(normalization_code, model::NotebookDiagnosticCode, {
    InvalidJson => "invalid_json", InvalidNotebook => "invalid_notebook",
    InvalidCells => "invalid_cells", InvalidCell => "invalid_cell",
    InvalidCellType => "invalid_cell_type", InvalidCellId => "invalid_cell_id",
    InvalidSource => "invalid_source", InvalidMetadata => "invalid_metadata",
    DuplicateEffectiveId => "duplicate_effective_id", SourceDerivedIdentity => "source_derived_identity",
    RepeatedAnonymousSource => "repeated_anonymous_source", UnsupportedLanguage => "unsupported_language",
    UnsupportedRustCommand => "unsupported_rust_command", UnsupportedRustDependency => "unsupported_rust_dependency",
    NonLeadingRustDirective => "non_leading_rust_directive", UnsupportedPythonMagic => "unsupported_python_magic"
});
enum_name!(symbol_kind, core::NotebookSymbolChangeKind, {
    Added => "added", Modified => "modified", Deleted => "deleted", Renamed => "renamed"
});
enum_name!(cell_kind, core::NotebookCellChangeKind, {
    Added => "added", Deleted => "deleted", Moved => "moved", TypeChanged => "type_changed",
    LanguageChanged => "language_changed", SourceChanged => "source_changed", MetadataChanged => "metadata_changed",
    DependencyDirectivesChanged => "dependency_directives_changed"
});
enum_name!(document_kind, core::NotebookDocumentChangeKind, {
    Added => "added", Deleted => "deleted", Renamed => "renamed", MetadataChanged => "metadata_changed"
});

impl From<core::SymbolDiagnostic> for Diagnostic {
    fn from(d: core::SymbolDiagnostic) -> Self {
        use core::SymbolDiagnosticCode as C;
        let code = match d.code {
            C::Normalization(code) => normalization_code(code),
            C::InvalidPath => "invalid_path",
            C::MissingSide => "missing_side",
            C::UnexpectedSide => "unexpected_side",
            C::UnsupportedChangeKind => "unsupported_change_kind",
            C::UnsupportedCellLanguage => "unsupported_cell_language",
            C::AmbiguousSymbolIdentity => "ambiguous_symbol_identity",
            C::AmbiguousCellIdentity => "ambiguous_cell_identity",
            C::AmbiguousRename => "ambiguous_rename",
            C::SnapshotIdentityCollision => "snapshot_identity_collision",
        };
        Self {
            code: code.into(),
            side: d.side.map(|s| {
                match s {
                    core::NotebookSide::Before => "before",
                    core::NotebookSide::After => "after",
                }
                .into()
            }),
            cell_ordinal: d.cell_ordinal.map(|n| n as u64),
            byte_start: d.byte_range.map(|r| r[0] as u64),
            byte_end: d.byte_range.map(|r| r[1] as u64),
            message: d.message,
        }
    }
}

fn list<T, U: Into<T>>(items: impl IntoIterator<Item = U>) -> List<T> {
    items.into_iter().map(|item| Some(item.into())).collect()
}

impl From<core::SourceRef> for SourceRef {
    fn from(r: core::SourceRef) -> Self {
        let coordinate_space = match r.coordinate_space {
            core::CoordinateSpace::DecodedCodeCellUtf8 => "decoded_code_cell_utf8",
        };
        Self {
            notebook_path: r.notebook_path,
            notebook_source_sha256: r.notebook_source_sha256,
            cell_id: r.cell_id,
            cell_ordinal: r.cell_ordinal as u64,
            cell_identity_quality: identity(r.cell_identity_quality).into(),
            cell_source_sha256: r.cell_source_sha256,
            cell_language: format!("{:?}", r.cell_language),
            coordinate_space: coordinate_space.into(),
            byte_start: r.byte_range[0] as u64,
            byte_end: r.byte_range[1] as u64,
            line_start: r.line_range[0] as u64,
            line_end: r.line_range[1] as u64,
        }
    }
}
impl From<core::NotebookSymbol> for Symbol {
    fn from(s: core::NotebookSymbol) -> Self {
        Self {
            id: s.id,
            symbol: s.symbol.into(),
            source_ref: s.source_ref.into(),
        }
    }
}
impl From<model::PreparedSource> for PreparedSource {
    fn from(s: model::PreparedSource) -> Self {
        let coordinate_map = match s.coordinate_map {
            model::SourceCoordinateMap::Identity => "identity",
        };
        Self {
            original: s.original,
            original_sha256: s.original_sha256,
            analysis: s.analysis,
            analysis_sha256: s.analysis_sha256,
            policy: s.policy.into(),
            coordinate_map: coordinate_map.into(),
            directives: list(s.directives.into_iter().map(|d| Directive {
                text: d.text,
                byte_start: d.byte_range[0] as u64,
                byte_end: d.byte_range[1] as u64,
            })),
        }
    }
}
impl From<core::NotebookCellSymbols> for Cell {
    fn from(c: core::NotebookCellSymbols) -> Self {
        let cell = c.cell;
        Self {
            id: cell.id,
            identity_quality: identity(cell.identity_quality).into(),
            ordinal: cell.ordinal as u64,
            cell_type: cell.cell_type,
            language: cell.language.map(|l| format!("{l:?}")),
            metadata_json: cell.metadata.to_string(),
            source: cell.source.map(Into::into),
            status: status(cell.status).into(),
            diagnostics: list(cell.diagnostics.into_iter().map(|d| Diagnostic {
                code: normalization_code(d.code).into(),
                side: None,
                cell_ordinal: d.cell_ordinal.map(|n| n as u64),
                byte_start: d.byte_range.map(|r| r[0] as u64),
                byte_end: d.byte_range.map(|r| r[1] as u64),
                message: d.message,
            })),
            symbols: list(c.symbols),
        }
    }
}
impl From<core::NotebookSymbolDocument> for Document {
    fn from(d: core::NotebookSymbolDocument) -> Self {
        Self {
            contract_version: d.contract_version,
            path: d.path,
            source_sha256: d.source_sha256,
            metadata_json: d.metadata.to_string(),
            cells: list(d.cells),
            status: status(d.status).into(),
            diagnostics: list(d.diagnostics),
        }
    }
}
impl From<core::CellRef> for CellRef {
    fn from(r: core::CellRef) -> Self {
        Self {
            notebook_path: r.notebook_path,
            notebook_source_sha256: r.notebook_source_sha256,
            cell_id: r.cell_id,
            cell_ordinal: r.cell_ordinal as u64,
            cell_source_sha256: r.cell_source_sha256,
            language: r.language.map(|l| format!("{l:?}")),
            cell_type: r.cell_type,
        }
    }
}
impl From<core::NotebookComparison> for Comparison {
    fn from(c: core::NotebookComparison) -> Self {
        Self {
            contract_version: c.contract_version,
            status: status(c.status).into(),
            diagnostics: list(c.diagnostics),
            before: c.before.map(Into::into),
            after: c.after.map(Into::into),
            symbol_changes: list(c.symbol_changes.into_iter().map(|c| SymbolChange {
                kind: symbol_kind(c.kind).into(),
                before: c.before.map(Into::into),
                after: c.after.map(Into::into),
            })),
            cell_changes: list(c.cell_changes.into_iter().map(|c| CellChange {
                kind: cell_kind(c.kind).into(),
                before: c.before.map(Into::into),
                after: c.after.map(Into::into),
            })),
            document_changes: list(c.document_changes.into_iter().map(|c| DocumentChange {
                kind: document_kind(c.kind).into(),
                before_path: c.before_path,
                after_path: c.after_path,
                before_metadata_json: c.before_metadata.map(|v| v.to_string()),
                after_metadata_json: c.after_metadata.map(|v| v.to_string()),
            })),
        }
    }
}

fn missing_children() -> Error {
    Error::internal("spur_graph_notebook: nested vector children missing")
}

fn reserve_list<T: NativeElement>(
    fields: &mut ChildrenMut<'_>,
    index: usize,
    rows: &[&T],
    reserve: impl FnOnce(&mut ChildrenMut<'_>, &[&T]) -> Result<()>,
) -> Result<()> {
    let mut vector = fields.get_vector_at_mut::<List<T>>(index)?;
    vector.set_child_size(rows.len())?;
    let mut elements = vector.children_mut().ok_or_else(missing_children)?;
    let mut elements = elements.get_vector_at_mut::<T>(0)?;
    reserve(
        &mut elements.children_mut().ok_or_else(missing_children)?,
        rows,
    )
}

fn reserve_struct<T: NativeElement>(
    fields: &mut ChildrenMut<'_>,
    index: usize,
    rows: &[&T],
    reserve: impl FnOnce(&mut ChildrenMut<'_>, &[&T]) -> Result<()>,
) -> Result<()> {
    let mut vector = fields.get_vector_at_mut::<T>(index)?;
    reserve(
        &mut vector.children_mut().ok_or_else(missing_children)?,
        rows,
    )
}

fn reserve_symbols(fields: &mut ChildrenMut<'_>, rows: &[&Symbol]) -> Result<()> {
    let count = rows
        .iter()
        .try_fold(0usize, |n, s| n.checked_add(s.symbol.tokens.0.len()))
        .ok_or_else(|| Error::internal("spur_graph_notebook: token count overflow"))?;
    fields
        .get_vector_at_mut::<SymbolFact>(1)?
        .children_mut()
        .ok_or_else(missing_children)?
        .get_vector_at_mut::<List<String>>(8)?
        .set_child_size(count)
}

fn reserve_cells(fields: &mut ChildrenMut<'_>, rows: &[&Cell]) -> Result<()> {
    let diagnostics = rows.iter().map(|c| c.diagnostics.0.len()).sum();
    fields
        .get_vector_at_mut::<List<Diagnostic>>(8)?
        .set_child_size(diagnostics)?;
    let sources: Vec<_> = rows.iter().filter_map(|c| c.source.as_ref()).collect();
    reserve_struct(fields, 6, &sources, |fields, rows| {
        fields
            .get_vector_at_mut::<List<Directive>>(6)?
            .set_child_size(rows.iter().map(|s| s.directives.0.len()).sum())
    })?;
    let symbols: Vec<_> = rows
        .iter()
        .flat_map(|c| c.symbols.0.iter().flatten())
        .collect();
    reserve_list(fields, 9, &symbols, reserve_symbols)
}

fn reserve_documents(fields: &mut ChildrenMut<'_>, rows: &[&Document]) -> Result<()> {
    let diagnostics = rows.iter().map(|d| d.diagnostics.0.len()).sum();
    fields
        .get_vector_at_mut::<List<Diagnostic>>(6)?
        .set_child_size(diagnostics)?;
    let cells: Vec<_> = rows
        .iter()
        .flat_map(|d| d.cells.0.iter().flatten())
        .collect();
    reserve_list(fields, 4, &cells, reserve_cells)
}

fn reserve_comparisons(fields: &mut ChildrenMut<'_>, rows: &[&Comparison]) -> Result<()> {
    fields
        .get_vector_at_mut::<List<Diagnostic>>(2)?
        .set_child_size(rows.iter().map(|c| c.diagnostics.0.len()).sum())?;
    for index in [3, 4] {
        let docs: Vec<_> = rows
            .iter()
            .filter_map(|c| {
                if index == 3 {
                    c.before.as_ref()
                } else {
                    c.after.as_ref()
                }
            })
            .collect();
        reserve_struct(fields, index, &docs, reserve_documents)?;
    }
    let changes: Vec<_> = rows
        .iter()
        .flat_map(|c| c.symbol_changes.0.iter().flatten())
        .collect();
    reserve_list(fields, 5, &changes, |fields, rows| {
        for index in [1, 2] {
            let symbols: Vec<_> = rows
                .iter()
                .filter_map(|c| {
                    if index == 1 {
                        c.before.as_ref()
                    } else {
                        c.after.as_ref()
                    }
                })
                .collect();
            reserve_struct(fields, index, &symbols, reserve_symbols)?;
        }
        Ok(())
    })?;
    fields
        .get_vector_at_mut::<List<CellChange>>(6)?
        .set_child_size(rows.iter().map(|c| c.cell_changes.0.len()).sum())?;
    fields
        .get_vector_at_mut::<List<DocumentChange>>(7)?
        .set_child_size(rows.iter().map(|c| c.document_changes.0.len()).sum())
}

struct ParseNotebook;
impl ScalarCallbacks for ParseNotebook {
    type BindData = ();
    type InitData = ();
    fn exec(
        &self,
        _context: &Context,
        _bind: Option<&()>,
        _init: Option<&mut ()>,
        vectors: &mut VectorCollection,
        output: Vector<'_, Unknown>,
    ) -> Result<()> {
        let paths = vectors.get_vector_at::<String>(0)?;
        let sources = vectors.get_vector_at::<String>(1)?;
        let paths = paths.reader()?;
        let sources = sources.reader()?;
        let mut rows: Vec<Option<Document>> = Vec::with_capacity(vectors.row_count()?);
        for index in 0..vectors.row_count()? {
            rows.push(match (paths.get(index)?, sources.get(index)?) {
                (Some(path), Some(source)) => Some(
                    core::extract_notebook_symbols(path, source.as_bytes())
                        .map_err(|e| {
                            Error::internal(format!("spur_graph_notebook_symbols: {e:#}"))
                        })?
                        .into(),
                ),
                _ => None,
            });
        }
        let mut output = output.cast::<Document>()?;
        output.set_size(rows.len())?;
        // Allocate every LIST descendant before any writer borrows the buffers,
        // including children of absent optional structs and zero-length lists.
        reserve_documents(
            &mut output.children_mut().ok_or_else(missing_children)?,
            &rows.iter().flatten().collect::<Vec<_>>(),
        )?;
        let mut writer = output.row_writer()?;
        for (index, row) in rows.iter().enumerate() {
            writer.set_option(index, row.as_ref())?;
        }
        Ok(())
    }
}

struct CompareNotebooks;
impl ScalarCallbacks for CompareNotebooks {
    type BindData = ();
    type InitData = ();
    fn exec(
        &self,
        _context: &Context,
        _bind: Option<&()>,
        _init: Option<&mut ()>,
        vectors: &mut VectorCollection,
        output: Vector<'_, Unknown>,
    ) -> Result<()> {
        let paths = vectors.get_vector_at::<String>(0)?;
        let old_paths = vectors.get_vector_at::<String>(1)?;
        let kinds = vectors.get_vector_at::<String>(2)?;
        let left = vectors.get_vector_at::<String>(3)?;
        let right = vectors.get_vector_at::<String>(4)?;
        let paths = paths.reader()?;
        let old_paths = old_paths.reader()?;
        let kinds = kinds.reader()?;
        let left = left.reader()?;
        let right = right.reader()?;
        let mut rows: Vec<Option<Comparison>> = Vec::with_capacity(vectors.row_count()?);
        for index in 0..vectors.row_count()? {
            rows.push(match (paths.get(index)?, kinds.get(index)?) {
                (Some(path), Some(kind)) => Some(
                    core::compare_notebook_sources(
                        path,
                        old_paths.get(index)?,
                        kind,
                        left.get(index)?.map(str::as_bytes),
                        right.get(index)?.map(str::as_bytes),
                    )
                    .map_err(|e| Error::internal(format!("spur_graph_notebook_changes: {e:#}")))?
                    .into(),
                ),
                _ => None,
            });
        }
        let mut output = output.cast::<Comparison>()?;
        output.set_size(rows.len())?;
        reserve_comparisons(
            &mut output.children_mut().ok_or_else(missing_children)?,
            &rows.iter().flatten().collect::<Vec<_>>(),
        )?;
        let mut writer = output.row_writer()?;
        for (index, row) in rows.iter().enumerate() {
            writer.set_option(index, row.as_ref())?;
        }
        Ok(())
    }
}

pub(crate) fn register(extension: &Extension) -> Result<()> {
    let contract = "contract_version=1; typed cell coverage complete/partial/invalid and diagnostics; metadata_json is opaque; original SourceRefs use decoded_code_cell_utf8; Rust leading :dep masking preserves offsets, never executes directives";
    ScalarFunctionBuilder::new("spur_graph_notebook_symbols", SignatureBuilder::new()
        .param(FunctionParameter::new::<String>("path"))
        .param(FunctionParameter::new::<String>("source"))
        .return_type(Document::logical_type(extension)?)
        .documentation(FunctionDocumentationBuilder::new()
            .description(format!("Extract cell-aware notebook symbols; {contract}; NULL path/source returns NULL; invalid JSON is typed invalid coverage; infrastructure errors propagate"))
            .add_example("Keep notebook inventory", "SELECT spur_graph_notebook_symbols('n.ipynb', '{\"cells\":[]}')")
            .add_category("spur_graph")), ParseNotebook)
        .set_property(FunctionProperty::HasSpecialNullHandling(true)).register(extension)?;
    ScalarFunctionBuilder::new("spur_graph_notebook_changes", SignatureBuilder::new()
        .param(FunctionParameter::new::<String>("path"))
        .param(FunctionParameter::new::<String>("old_path"))
        .param(FunctionParameter::new::<String>("kind"))
        .param(FunctionParameter::new::<String>("left_source"))
        .param(FunctionParameter::new::<String>("right_source"))
        .return_type(Comparison::logical_type(extension)?)
        .documentation(FunctionDocumentationBuilder::new()
            .description(format!("Compare paired notebook cells; {contract}; NULL path/kind returns NULL; M requires both sources, A absent left, D absent right; R/R100 requires distinct old_path, otherwise old_path defaults to path; missing/unexpected sides are invalid, other kinds partial; infrastructure errors propagate"))
            .add_example("Compare notebook sources", "SELECT spur_graph_notebook_changes('n.ipynb', NULL, 'M', '{\"cells\":[]}', '{\"cells\":[]}')")
            .add_category("spur_graph")), CompareNotebooks)
        .set_property(FunctionProperty::HasSpecialNullHandling(true)).register(extension)
}
