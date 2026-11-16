//! Typed native one-file facts. Numeric IDs belong to one input row: joins
//! must retain the caller's input key. Resolution never sees another file.

use std::collections::{hash_map::Entry, HashMap};
use std::path::Path;

use base64::{engine::general_purpose::STANDARD, Engine as _};
use duckdb_neo::connection::Context;
use duckdb_neo::data_chunk::{ChildrenMut, VectorCollection};
use duckdb_neo::error::Error;
use duckdb_neo::extension::Extension;
use duckdb_neo::scalar::{ScalarCallbacks, ScalarFunctionBuilder, ScalarInitCallbacks};
use duckdb_neo::signature::{FunctionDocumentationBuilder, FunctionParameter, SignatureBuilder};
use duckdb_neo::types::List;
use duckdb_neo::vector::{RowWriter, Unknown, Vector};
use duckdb_neo::{AsDuckDB, DuckDBType, Result};
use spur_graph::extract::languages::Language;
use spur_graph::extract::tree_sitter::BytesExtractor;
use spur_graph::{
    ChangeKind, Confidence, GitPath, GraphEdge, GraphEdgeKind, GraphFacts, GraphNode,
    MacroAttributeEvidence, MacroDefinitionLocation, MacroExecutionPhase, MacroInvocationEvidence,
    MacroResolutionEvidence, MacroScopeEvidence, MacroScopeKind, MacroScopeRef, MacroSinkFamily,
    MacroTestContext, RelationKind, RenamePrev, SnapshotKey, SourceSpan,
};

#[derive(AsDuckDB)]
#[duckdb(writer)]
struct FileFacts {
    nodes: List<Node>,
    edges: List<Edge>,
    spans: List<Span>,
    macro_invocations: List<Invocation>,
    macro_scopes: List<ScopeEvidence>,
}

#[derive(AsDuckDB)]
#[duckdb(writer)]
struct Node {
    node_id: u64,
    stable_key: String,
    label: String,
    kind: String,
    file_id: Option<u64>,
    source_span_id: Option<u64>,
    first_seen_run_id: u64,
}

#[derive(AsDuckDB)]
#[duckdb(writer)]
struct Edge {
    edge_id: u64,
    source_node_id: u64,
    target_node_id: Option<u64>,
    relation: String,
    target_label: Option<String>,
    import_path: Option<String>,
    receiver_text: Option<String>,
    scope_text: Option<String>,
    confidence: String,
    confidence_score: f32,
    edge_kind: Option<String>,
    bind_method: Option<String>,
    evidence_id: u64,
    directed: bool,
    change_kind: Option<Change>,
}

#[derive(AsDuckDB)]
#[duckdb(writer)]
struct Span {
    span_id: u64,
    file_id: u64,
    start_byte: u32,
    end_byte: u32,
    start_line: u32,
    end_line: u32,
}

#[derive(AsDuckDB)]
#[duckdb(writer)]
struct Scope {
    stable_symbol_id: String,
    kind: String,
}

#[derive(AsDuckDB)]
#[duckdb(writer)]
struct Location {
    file_path: String,
    source_oid: String,
    byte_range: List<u64>,
    line_range: List<u64>,
}

#[derive(AsDuckDB)]
#[duckdb(writer)]
struct Attribute {
    raw_text: String,
    owner: Scope,
    byte_range: List<u64>,
    line_range: List<u64>,
}

#[derive(AsDuckDB)]
#[duckdb(writer)]
struct Resolution {
    kind: String,
    definition: Option<Location>,
}

#[derive(AsDuckDB)]
#[duckdb(writer)]
struct Invocation {
    site_id: String,
    file_path: String,
    source_oid: String,
    caller_stable_symbol_id: Option<String>,
    enclosing: Scope,
    raw_macro_path: String,
    normalized_macro_path: String,
    byte_range: List<u64>,
    line_range: List<u64>,
    attributes: List<Attribute>,
    test_context: String,
    debug_assertions_candidate: bool,
    execution_phase: String,
    resolution: Resolution,
    sink_family_candidate: Option<String>,
}

#[derive(AsDuckDB)]
#[duckdb(writer)]
struct ScopeEvidence {
    scope: Scope,
    file_path: String,
    source_oid: String,
    byte_range: List<u64>,
    line_range: List<u64>,
}

// Lossless normalization of the native externally tagged temporal enums.
// Inactive payloads are NULL. GitPath preserves the native UTF-8 or base64 form.
#[derive(AsDuckDB)]
#[duckdb(writer)]
struct Change {
    kind: String,
    renamed_from: Option<Previous>,
}

#[derive(AsDuckDB)]
#[duckdb(writer)]
struct Previous {
    kind: String,
    file: Option<PathValue>,
    symbol: Option<Snapshot>,
}

#[derive(AsDuckDB)]
#[duckdb(writer)]
struct PathValue {
    path: Option<String>,
    encoding: Option<String>,
    bytes: Option<String>,
}

#[derive(AsDuckDB)]
#[duckdb(writer)]
struct Snapshot {
    stable_symbol_id: String,
    commit: String,
}

trait NativeLabel {
    fn label(self) -> &'static str;
}

// Exhaustive matches make new native variants a compile-time mapping decision.
macro_rules! labels {
    ($ty:ident { $($variant:ident => $label:literal),* $(,)? }) => {
        impl NativeLabel for $ty {
            fn label(self) -> &'static str {
                match self { $(Self::$variant => $label),* }
            }
        }
    };
}

labels!(RelationKind {
    Imports => "imports", Calls => "calls", Constructs => "constructs",
    Contains => "contains", Implements => "implements", Defines => "defines",
    References => "references", Extends => "extends", Links => "links",
    Touches => "touches", Produces => "produces", Consumes => "consumes",
    Binds => "binds", Emits => "emits",
});
labels!(Confidence {
    SyntaxExact => "syntax_exact", Heuristic => "heuristic", Unknown => "unknown",
});
labels!(GraphEdgeKind {
    Calls => "calls", CallsDyn => "calls_dyn", ReferencesHof => "references_hof",
    ReferencesOther => "references_other", ReferencesAddress => "references_address",
});
labels!(MacroScopeKind {
    File => "file", Module => "module", Function => "function", Const => "const",
    Static => "static", Type => "type", Impl => "impl", Unknown => "unknown",
});
labels!(MacroTestContext {
    Unspecified => "unspecified", TestAttributeCandidate => "test_attribute_candidate",
    Conditional => "conditional",
});
labels!(MacroExecutionPhase {
    Unknown => "unknown", FunctionBodySyntax => "function_body_syntax",
    ItemSyntax => "item_syntax", ConstContextSyntax => "const_context_syntax",
});
labels!(MacroSinkFamily {
    Panic => "panic", Assertion => "assertion", DebugAssertion => "debug_assertion",
});

fn label(value: impl NativeLabel) -> String {
    value.label().to_owned()
}

fn list<T, U: From<T>>(values: Vec<T>) -> List<U> {
    values.into_iter().map(|value| Some(value.into())).collect()
}

fn range(value: [usize; 2]) -> List<u64> {
    value
        .into_iter()
        .map(|coordinate| Some(coordinate as u64))
        .collect()
}

impl From<GraphFacts> for FileFacts {
    fn from(facts: GraphFacts) -> Self {
        Self {
            nodes: list(facts.nodes),
            edges: list(facts.edges),
            spans: list(facts.spans),
            macro_invocations: list(facts.macro_invocations),
            macro_scopes: list(facts.macro_scopes),
        }
    }
}

impl From<GraphNode> for Node {
    fn from(node: GraphNode) -> Self {
        Self {
            node_id: node.node_id.get(),
            stable_key: node.stable_key,
            label: node.label,
            kind: node.kind.discriminator().to_owned(),
            file_id: node.file_id.map(|id| id.get()),
            source_span_id: node.source_span_id.map(|id| id.get()),
            first_seen_run_id: node.first_seen_run_id.get(),
        }
    }
}

impl From<GraphEdge> for Edge {
    fn from(edge: GraphEdge) -> Self {
        Self {
            edge_id: edge.edge_id.get(),
            source_node_id: edge.source_node_id.get(),
            target_node_id: edge.target_node_id.map(|id| id.get()),
            relation: label(edge.relation),
            target_label: edge.target_label,
            import_path: edge.import_path,
            receiver_text: edge.receiver_text,
            scope_text: edge.scope_text,
            confidence: label(edge.confidence),
            confidence_score: edge.confidence_score,
            edge_kind: edge.edge_kind.map(label),
            bind_method: edge.bind_method,
            evidence_id: edge.evidence_id.get(),
            directed: edge.directed,
            change_kind: edge.change_kind.map(Into::into),
        }
    }
}

impl From<SourceSpan> for Span {
    fn from(span: SourceSpan) -> Self {
        Self {
            span_id: span.span_id.get(),
            file_id: span.file_id.get(),
            start_byte: span.start_byte,
            end_byte: span.end_byte,
            start_line: span.start_line,
            end_line: span.end_line,
        }
    }
}

impl From<MacroScopeRef> for Scope {
    fn from(scope: MacroScopeRef) -> Self {
        Self {
            stable_symbol_id: scope.stable_symbol_id,
            kind: label(scope.kind),
        }
    }
}

impl From<MacroDefinitionLocation> for Location {
    fn from(location: MacroDefinitionLocation) -> Self {
        Self {
            file_path: location.file_path,
            source_oid: location.source_oid,
            byte_range: range(location.byte_range),
            line_range: range(location.line_range),
        }
    }
}

impl From<MacroAttributeEvidence> for Attribute {
    fn from(attribute: MacroAttributeEvidence) -> Self {
        Self {
            raw_text: attribute.raw_text,
            owner: attribute.owner.into(),
            byte_range: range(attribute.byte_range),
            line_range: range(attribute.line_range),
        }
    }
}

impl From<MacroResolutionEvidence> for Resolution {
    fn from(resolution: MacroResolutionEvidence) -> Self {
        let (kind, definition) = match resolution {
            MacroResolutionEvidence::Unresolved => ("unresolved", None),
            MacroResolutionEvidence::ExplicitPathCandidate => ("explicit_path_candidate", None),
            MacroResolutionEvidence::LocalDefinitionCandidate { definition } => {
                ("local_definition_candidate", Some(definition.into()))
            }
        };
        Self {
            kind: kind.to_owned(),
            definition,
        }
    }
}

impl From<MacroInvocationEvidence> for Invocation {
    fn from(invocation: MacroInvocationEvidence) -> Self {
        Self {
            site_id: invocation.site_id,
            file_path: invocation.file_path,
            source_oid: invocation.source_oid,
            caller_stable_symbol_id: invocation.caller_stable_symbol_id,
            enclosing: invocation.enclosing.into(),
            raw_macro_path: invocation.raw_macro_path,
            normalized_macro_path: invocation.normalized_macro_path,
            byte_range: range(invocation.byte_range),
            line_range: range(invocation.line_range),
            attributes: list(invocation.attributes),
            test_context: label(invocation.test_context),
            debug_assertions_candidate: invocation.debug_assertions_candidate,
            execution_phase: label(invocation.execution_phase),
            resolution: invocation.resolution.into(),
            sink_family_candidate: invocation.sink_family_candidate.map(label),
        }
    }
}

impl From<MacroScopeEvidence> for ScopeEvidence {
    fn from(evidence: MacroScopeEvidence) -> Self {
        Self {
            scope: evidence.scope.into(),
            file_path: evidence.file_path,
            source_oid: evidence.source_oid,
            byte_range: range(evidence.byte_range),
            line_range: range(evidence.line_range),
        }
    }
}

impl From<ChangeKind> for Change {
    fn from(change: ChangeKind) -> Self {
        let (kind, renamed_from) = match change {
            ChangeKind::Added => ("added", None),
            ChangeKind::Modified => ("modified", None),
            ChangeKind::Deleted => ("deleted", None),
            ChangeKind::RenamedFrom(previous) => ("renamed_from", Some(previous.into())),
        };
        Self {
            kind: kind.to_owned(),
            renamed_from,
        }
    }
}

impl From<RenamePrev> for Previous {
    fn from(previous: RenamePrev) -> Self {
        match previous {
            RenamePrev::File(path) => Self {
                kind: "file".into(),
                file: Some(path.into()),
                symbol: None,
            },
            RenamePrev::Symbol(snapshot) => Self {
                kind: "symbol".into(),
                file: None,
                symbol: Some(snapshot.into()),
            },
        }
    }
}

impl From<GitPath> for PathValue {
    fn from(path: GitPath) -> Self {
        match String::from_utf8(path.into_bytes()) {
            Ok(path) => Self {
                path: Some(path),
                encoding: None,
                bytes: None,
            },
            Err(error) => Self {
                path: None,
                encoding: Some("base64".into()),
                bytes: Some(STANDARD.encode(error.into_bytes())),
            },
        }
    }
}

impl From<SnapshotKey> for Snapshot {
    fn from(key: SnapshotKey) -> Self {
        Self {
            stable_symbol_id: key.stable_symbol_id,
            commit: key.commit,
        }
    }
}

fn missing_children() -> Error {
    Error::internal("spur_graph_file_facts: nested vector children missing")
}

fn reserve_ranges(fields: &mut ChildrenMut<'_>, indices: [usize; 2], rows: usize) -> Result<()> {
    let coordinates = rows
        .checked_mul(2)
        .ok_or_else(|| Error::invalid_input("spur_graph_file_facts: range child size overflow"))?;
    for index in indices {
        fields
            .get_vector_at_mut::<List<u64>>(index)?
            .set_child_size(coordinates)?;
    }
    Ok(())
}

/// Size every LIST descendant before creating any writer. Struct child row
/// counts follow their parent; only list elements have independent counts.
/// Indices below follow the typed structs above and each access checks its type.
fn reserve_output(output: &mut Vector<'_, FileFacts>, rows: &[Option<FileFacts>]) -> Result<()> {
    let facts = || rows.iter().flatten();
    let node_count = facts().map(|row| row.nodes.0.len()).sum();
    let edge_count = facts().map(|row| row.edges.0.len()).sum();
    let span_count = facts().map(|row| row.spans.0.len()).sum();
    let invocation_count = facts().map(|row| row.macro_invocations.0.len()).sum();
    let scope_count = facts().map(|row| row.macro_scopes.0.len()).sum();
    let invocations = || facts().flat_map(|row| row.macro_invocations.0.iter().flatten());
    let attribute_count = invocations().map(|row| row.attributes.0.len()).sum();
    let definition_count = invocations()
        .filter(|row| row.resolution.definition.is_some())
        .count();

    output.set_size(rows.len())?;
    let mut fields = output.children_mut().ok_or_else(missing_children)?;
    fields
        .get_vector_at_mut::<List<Node>>(0)?
        .set_child_size(node_count)?;
    fields
        .get_vector_at_mut::<List<Edge>>(1)?
        .set_child_size(edge_count)?;
    fields
        .get_vector_at_mut::<List<Span>>(2)?
        .set_child_size(span_count)?;
    {
        let mut list = fields.get_vector_at_mut::<List<Invocation>>(3)?;
        list.set_child_size(invocation_count)?;
        let mut elements = list.children_mut().ok_or_else(missing_children)?;
        let mut invocations = elements.get_vector_at_mut::<Invocation>(0)?;
        let mut fields = invocations.children_mut().ok_or_else(missing_children)?;
        reserve_ranges(&mut fields, [7, 8], invocation_count)?;
        {
            let mut list = fields.get_vector_at_mut::<List<Attribute>>(9)?;
            list.set_child_size(attribute_count)?;
            let mut elements = list.children_mut().ok_or_else(missing_children)?;
            let mut attributes = elements.get_vector_at_mut::<Attribute>(0)?;
            reserve_ranges(
                &mut attributes.children_mut().ok_or_else(missing_children)?,
                [2, 3],
                attribute_count,
            )?;
        }
        {
            let mut resolution = fields.get_vector_at_mut::<Resolution>(13)?;
            let mut fields = resolution.children_mut().ok_or_else(missing_children)?;
            // Optional structs still own child vectors, including when every
            // definition is NULL. Their range element storage is independent.
            let mut definition = fields.get_vector_at_mut::<Location>(1)?;
            reserve_ranges(
                &mut definition.children_mut().ok_or_else(missing_children)?,
                [2, 3],
                definition_count,
            )?;
        }
    }
    {
        let mut list = fields.get_vector_at_mut::<List<ScopeEvidence>>(4)?;
        list.set_child_size(scope_count)?;
        let mut elements = list.children_mut().ok_or_else(missing_children)?;
        let mut scopes = elements.get_vector_at_mut::<ScopeEvidence>(0)?;
        reserve_ranges(
            &mut scopes.children_mut().ok_or_else(missing_children)?,
            [3, 4],
            scope_count,
        )?;
    }
    Ok(())
}

struct ParseFileFacts;

impl ScalarInitCallbacks for ParseFileFacts {
    fn init(
        &self,
        _context: &Context,
        _bind_data: Option<&Self::BindData>,
    ) -> Result<Self::InitData> {
        Ok(HashMap::new())
    }
}

impl ScalarCallbacks for ParseFileFacts {
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
        let parsers = init_data
            .ok_or_else(|| Error::internal("spur_graph_file_facts: missing worker state"))?;
        let paths = vectors.get_vector_at::<String>(0)?;
        let sources = vectors.get_vector_at::<String>(1)?;
        let paths = paths.reader()?;
        let sources = sources.reader()?;
        let mut rows = Vec::with_capacity(vectors.row_count()?);
        for index in 0..vectors.row_count()? {
            let (Some(path), Some(source)) = (paths.get(index)?, sources.get(index)?) else {
                rows.push(None);
                continue;
            };
            let path = Path::new(path);
            let language = Language::from_path(path).ok_or_else(|| {
                Error::invalid_input(format!(
                    "spur_graph_file_facts: unsupported language for path: {}",
                    path.display()
                ))
            })?;
            if let Entry::Vacant(entry) = parsers.entry(language) {
                entry.insert(BytesExtractor::for_language(language).map_err(extraction_error)?);
            }
            let parser = parsers
                .get_mut(&language)
                .ok_or_else(|| Error::internal("spur_graph_file_facts: parser state missing"))?;
            // The pure core method owns path validation, a fresh builder/IDs,
            // notebook handling and native one-file pending-edge resolution.
            rows.push(Some(
                parser
                    .extract_file_facts(path, source.as_bytes())
                    .map_err(extraction_error)?
                    .into(),
            ));
        }
        // Extract first: a failing chunk leaves output untouched, including
        // when DuckDB's TRY retries individual rows after a batch error.
        let mut output = output.cast::<FileFacts>()?;
        reserve_output(&mut output, &rows)?;
        let mut writer = output.row_writer()?;
        for (index, row) in rows.iter().enumerate() {
            writer.set_option(index, row.as_ref())?;
        }
        Ok(())
    }
}

fn extraction_error(error: spur_graph::extract::tree_sitter::ExtractError) -> Error {
    Error::invalid_input(format!("spur_graph_file_facts: {error}"))
}

pub(crate) fn register(extension: &Extension) -> Result<()> {
    ScalarFunctionBuilder::new(
        "spur_graph_file_facts",
        SignatureBuilder::new()
            .param(FunctionParameter::new::<String>("logical_path"))
            .param(FunctionParameter::new::<String>("source"))
            .return_type(FileFacts::logical_type(extension)?)
            .documentation(FunctionDocumentationBuilder::new()
                .description("Extract native one-file graph facts; join local IDs with the input key and retain path/blob/version lineage")
                .add_category("spur_graph")),
        ParseFileFacts,
    ).with_init().register(extension)
}

#[cfg(test)]
mod diagnostic_tests {
    use super::*;
    use spur_graph::extract::tree_sitter::ExtractError;

    #[test]
    fn empty_facts_retains_native_payload_and_empty_source_takes_precedence() {
        for (empty_source, status) in [(false, "empty_facts"), (true, "empty_source")] {
            let result = FileFactsResult::success(GraphFacts::default(), empty_source);
            assert_eq!(result.status, status);
            assert!(result.error_code.is_none());
            assert!(result.error_message.is_none());
            let facts = result.facts.expect("empty native facts remain present");
            assert!(facts.nodes.0.is_empty());
            assert!(facts.edges.0.is_empty());
            assert!(facts.spans.0.is_empty());
            assert!(facts.macro_invocations.0.is_empty());
            assert!(facts.macro_scopes.0.is_empty());
        }
    }

    #[test]
    fn native_row_errors_keep_diagnostics_and_internal_errors_propagate() {
        let bytes = [0xff];
        let utf8 = std::str::from_utf8(&bytes).unwrap_err();
        for (error, code) in [
            (ExtractError::InvalidUtf8(utf8), "malformed_input"),
            (ExtractError::NoTree, "extraction_failed"),
            (ExtractError::Extraction("parse .ipynb JSON".into()), "malformed_input"),
            (ExtractError::Extraction("logical path must name a file".into()), "invalid_path"),
        ] {
            let message = error.to_string();
            let result = diagnostic_error(error).expect("recoverable native error");
            assert_eq!(result.status, "extraction_error");
            assert_eq!(result.error_code.as_deref(), Some(code));
            assert_eq!(result.error_message.as_deref(), Some(message.as_str()));
            assert!(result.facts.is_none());
        }
        for error in [
            ExtractError::Setup("query compiler failed".into()),
            ExtractError::Extraction("set rust parser language".into()),
            ExtractError::Extraction("unexpected kernel failure".into()),
        ] {
            let message = error.to_string();
            let error = diagnostic_error(error).err().expect("internal failure must propagate");
            assert!(error.to_string().contains(&message));
        }
    }
}
