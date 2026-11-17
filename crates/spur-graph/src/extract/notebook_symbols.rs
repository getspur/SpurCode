//! Cell-aware notebook symbol contract, version 1.
//!
//! Sources and ranges refer to original decoded cells. Matching is bounded to
//! unique identities inside paired cells and one normalized language. It reuses
//! the ordinary syntax/token matcher, not execution or semantic equivalence.
use super::{
    languages::Language,
    notebook_model::*,
    tree_sitter::{BytesExtractor, ExtractedSymbol},
};
use crate::git_walk::{
    snapshot_from, symbol_changes_from_extracted_with_language, FileChange, FileChangeKind,
};
use crate::{ChangeKind, GitPath, RenamePrev, SnapshotKey};
use serde_json::Value;
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    path::Path,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotebookSide {
    Before,
    After,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SymbolDiagnosticCode {
    Normalization(NotebookDiagnosticCode),
    InvalidPath,
    MissingSide,
    UnexpectedSide,
    UnsupportedChangeKind,
    UnsupportedCellLanguage,
    AmbiguousSymbolIdentity,
    AmbiguousCellIdentity,
    AmbiguousRename,
    SnapshotIdentityCollision,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SymbolDiagnostic {
    pub code: SymbolDiagnosticCode,
    pub side: Option<NotebookSide>,
    pub cell_ordinal: Option<usize>,
    pub byte_range: Option<[usize; 2]>,
    pub message: String,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoordinateSpace {
    DecodedCodeCellUtf8,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceRef {
    pub notebook_path: String,
    pub notebook_source_sha256: String,
    pub cell_id: String,
    pub cell_ordinal: usize,
    pub cell_identity_quality: CellIdentityQuality,
    pub cell_source_sha256: String,
    pub cell_language: Language,
    pub coordinate_space: CoordinateSpace,
    pub byte_range: [usize; 2],
    pub line_range: [usize; 2],
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotebookSymbol {
    pub id: String,
    pub symbol: ExtractedSymbol,
    pub source_ref: SourceRef,
}
#[derive(Debug, Clone, PartialEq)]
pub struct NotebookCellSymbols {
    pub cell: NormalizedCell,
    pub symbols: Vec<NotebookSymbol>,
}
#[derive(Debug, Clone, PartialEq)]
pub struct NotebookSymbolDocument {
    pub contract_version: u32,
    pub path: String,
    pub source_sha256: String,
    pub metadata: Value,
    pub cells: Vec<NotebookCellSymbols>,
    pub status: NotebookStatus,
    pub diagnostics: Vec<SymbolDiagnostic>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotebookSymbolChangeKind {
    Added,
    Modified,
    Deleted,
    Renamed,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotebookSymbolChange {
    pub kind: NotebookSymbolChangeKind,
    pub before: Option<NotebookSymbol>,
    pub after: Option<NotebookSymbol>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotebookCellChangeKind {
    Added,
    Deleted,
    Moved,
    TypeChanged,
    LanguageChanged,
    SourceChanged,
    MetadataChanged,
    DependencyDirectivesChanged,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CellRef {
    pub notebook_path: String,
    pub notebook_source_sha256: String,
    pub cell_id: String,
    pub cell_ordinal: usize,
    pub cell_source_sha256: Option<String>,
    pub language: Option<Language>,
    pub cell_type: Option<String>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotebookCellChange {
    pub kind: NotebookCellChangeKind,
    pub before: Option<CellRef>,
    pub after: Option<CellRef>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotebookDocumentChangeKind {
    Added,
    Deleted,
    Renamed,
    MetadataChanged,
}
#[derive(Debug, Clone, PartialEq)]
pub struct NotebookDocumentChange {
    pub kind: NotebookDocumentChangeKind,
    pub before_path: Option<String>,
    pub after_path: Option<String>,
    pub before_metadata: Option<Value>,
    pub after_metadata: Option<Value>,
}
#[derive(Debug, Clone, PartialEq)]
pub struct NotebookComparison {
    pub contract_version: u32,
    pub status: NotebookStatus,
    pub diagnostics: Vec<SymbolDiagnostic>,
    pub before: Option<NotebookSymbolDocument>,
    pub after: Option<NotebookSymbolDocument>,
    pub symbol_changes: Vec<NotebookSymbolChange>,
    pub cell_changes: Vec<NotebookCellChange>,
    pub document_changes: Vec<NotebookDocumentChange>,
}

/// Extract code-cell symbols with original decoded UTF-8 coordinates. Invalid
/// input is a typed Invalid document; extractor/setup failures propagate as Err.
/// Non-code cells remain in inventory. Complete means extraction coverage only,
/// never notebook execution or runtime dependency equivalence.
pub fn extract_notebook_symbols(
    path: &str,
    bytes: &[u8],
) -> anyhow::Result<NotebookSymbolDocument> {
    let nb = normalize_notebook(bytes);
    let mut doc = NotebookSymbolDocument {
        contract_version: 1,
        path: path.into(),
        source_sha256: nb.source_sha256,
        metadata: nb.metadata,
        cells: Vec::with_capacity(nb.cells.len()),
        status: NotebookStatus::Complete,
        diagnostics: nb
            .diagnostics
            .into_iter()
            .map(normalization_diagnostic)
            .collect(),
    };
    if !doc.diagnostics.is_empty() {
        doc.status = nb.status;
    }
    if !valid_path(path) {
        doc.status = NotebookStatus::Invalid;
        doc.diagnostics.push(diagnostic(
            SymbolDiagnosticCode::InvalidPath,
            None,
            "notebook path must be nonblank UTF-8 without NUL",
        ));
    }
    let mut parsers = HashMap::new();
    for mut cell in nb.cells {
        // Duplicate IDs are an identity limitation for this route. Other invalid
        // source/type/metadata shapes remain Invalid, exactly as normalized.
        if cell.identity_quality == CellIdentityQuality::Ambiguous {
            let invalid_shape = cell.diagnostics.iter().any(|d| {
                matches!(
                    d.code,
                    NotebookDiagnosticCode::InvalidCell
                        | NotebookDiagnosticCode::InvalidCellType
                        | NotebookDiagnosticCode::InvalidCellId
                        | NotebookDiagnosticCode::InvalidSource
                        | NotebookDiagnosticCode::InvalidMetadata
                )
            });
            if !invalid_shape {
                cell.status = NotebookStatus::Partial;
            }
        }
        doc.diagnostics.extend(
            cell.diagnostics
                .iter()
                .cloned()
                .map(normalization_diagnostic),
        );
        let mut symbols = Vec::new();
        if cell.cell_type.as_deref() == Some("code") {
            if let (Some(language), Some(source)) = (cell.language, cell.source.as_ref()) {
                if language == Language::JupyterNotebook {
                    cell.status = merge_status(cell.status, NotebookStatus::Partial);
                    doc.diagnostics.push(diagnostic(
                        SymbolDiagnosticCode::UnsupportedCellLanguage,
                        Some(cell.ordinal),
                        "nested notebook language is not a code-cell symbol extractor",
                    ));
                } else {
                    let parser = match parsers.entry(language) {
                        std::collections::hash_map::Entry::Occupied(entry) => entry.into_mut(),
                        std::collections::hash_map::Entry::Vacant(entry) => {
                            entry.insert(BytesExtractor::for_language(language)?)
                        }
                    };
                    let identity_path = canonical_cell_path(path, &cell.id);
                    for symbol in
                        parser.extract(Path::new(&identity_path), source.analysis.as_bytes())?
                    {
                        let [start, end] = symbol.byte_range;
                        anyhow::ensure!(
                            start <= end && source.original.get(start..end).is_some(),
                            "extractor range outside original decoded cell"
                        );
                        // Keep the ID preimage structured; no delimiter-joined FQN or
                        // finite hash serves as the new public identity.
                        let discriminator = (cell.identity_quality
                            == CellIdentityQuality::Ambiguous)
                            .then_some(cell.ordinal);
                        let id = format!(
                            "{identity_path}#symbol:{}",
                            serde_json::to_string(&(
                                language.label(),
                                &symbol.entity_name,
                                &symbol.symbol_kind,
                                &symbol.enclosing_scope,
                                start,
                                discriminator
                            ))?
                        );
                        let source_ref = SourceRef {
                            notebook_path: path.into(),
                            notebook_source_sha256: doc.source_sha256.clone(),
                            cell_id: cell.id.clone(),
                            cell_ordinal: cell.ordinal,
                            cell_identity_quality: cell.identity_quality,
                            cell_source_sha256: source.original_sha256.clone(),
                            cell_language: language,
                            coordinate_space: CoordinateSpace::DecodedCodeCellUtf8,
                            byte_range: symbol.byte_range,
                            line_range: symbol.line_range,
                        };
                        symbols.push(NotebookSymbol {
                            id,
                            symbol,
                            source_ref,
                        });
                    }
                }
            }
        }
        if duplicate_symbol_identities(&symbols) {
            cell.status = merge_status(cell.status, NotebookStatus::Partial);
            doc.diagnostics.push(diagnostic(SymbolDiagnosticCode::AmbiguousSymbolIdentity,
                Some(cell.ordinal), "same-cell (name, kind, scope) is not unique; all symbols retained, matching skipped"));
        }
        doc.status = merge_status(doc.status, cell.status);
        doc.cells.push(NotebookCellSymbols { cell, symbols });
    }
    Ok(doc)
}

/// Compare M/A/D/R/R100 notebook sources. A requires an absent left side; D an
/// absent right side; M and rename require both. Rename additionally requires a
/// distinct old_path. Other kinds are Partial with an explicit diagnostic.
/// Invalid inputs return Invalid and retain available inventory; internal errors
/// propagate. No snapshot/repository assertion is accepted or manufactured.
pub fn compare_notebook_sources(
    path: &str,
    old_path: Option<&str>,
    change_kind: &str,
    left_source: Option<&[u8]>,
    right_source: Option<&[u8]>,
) -> anyhow::Result<NotebookComparison> {
    let before = left_source
        .map(|b| extract_notebook_symbols(old_path.unwrap_or(path), b))
        .transpose()?;
    let after = right_source
        .map(|b| extract_notebook_symbols(path, b))
        .transpose()?;
    let mut result = NotebookComparison {
        contract_version: 1,
        status: NotebookStatus::Complete,
        diagnostics: Vec::new(),
        before,
        after,
        symbol_changes: Vec::new(),
        cell_changes: Vec::new(),
        document_changes: Vec::new(),
    };
    for (side, doc) in [
        (NotebookSide::Before, &result.before),
        (NotebookSide::After, &result.after),
    ] {
        if let Some(doc) = doc {
            result.status = merge_status(result.status, doc.status);
            result
                .diagnostics
                .extend(doc.diagnostics.iter().cloned().map(|mut d| {
                    d.side = Some(side);
                    d
                }));
        }
    }
    let rename = matches!(change_kind, "R" | "R100");
    let supported = matches!(change_kind, "M" | "A" | "D" | "R" | "R100");
    if !supported {
        result.status = merge_status(result.status, NotebookStatus::Partial);
        result.diagnostics.push(diagnostic(
            SymbolDiagnosticCode::UnsupportedChangeKind,
            None,
            "supported notebook change kinds are M, A, D, R and R100",
        ));
        return Ok(result);
    }
    if !valid_path(path)
        || (rename && old_path.is_none_or(|old| !valid_path(old) || old == path))
        || (!rename && old_path.is_some_and(|old| old != path))
    {
        result.status = NotebookStatus::Invalid;
        result.diagnostics.push(diagnostic(SymbolDiagnosticCode::InvalidPath, None,
            "rename requires a distinct valid old_path; other kinds require old_path absent or equal to path"));
    }
    for (side, required, present) in [
        (
            NotebookSide::Before,
            change_kind != "A",
            result.before.is_some(),
        ),
        (
            NotebookSide::After,
            change_kind != "D",
            result.after.is_some(),
        ),
    ] {
        if required != present {
            result.status = NotebookStatus::Invalid;
            let mut d = diagnostic(
                if required {
                    SymbolDiagnosticCode::MissingSide
                } else {
                    SymbolDiagnosticCode::UnexpectedSide
                },
                None,
                "change kind and source presence disagree",
            );
            d.side = Some(side);
            result.diagnostics.push(d);
        }
    }
    if result.status == NotebookStatus::Invalid {
        return Ok(result);
    }
    // Borrow documents separately while accumulating events. Inventories stay
    // intact, including unsupported and zero-symbol cells.
    let mut symbols = Vec::new();
    let mut cells = Vec::new();
    let mut documents = Vec::new();
    match (&result.before, &result.after) {
        (Some(before), Some(after)) => {
            if rename {
                documents.push(document_event(
                    NotebookDocumentChangeKind::Renamed,
                    Some(before),
                    Some(after),
                ));
            }
            if before.metadata != after.metadata {
                documents.push(document_event(
                    NotebookDocumentChangeKind::MetadataChanged,
                    Some(before),
                    Some(after),
                ));
            }
            let mut groups: BTreeMap<&str, (Vec<usize>, Vec<usize>)> = BTreeMap::new();
            for c in &before.cells {
                groups.entry(&c.cell.id).or_default().0.push(c.cell.ordinal);
            }
            for c in &after.cells {
                groups.entry(&c.cell.id).or_default().1.push(c.cell.ordinal);
            }
            for (_, (left, right)) in groups {
                let ambiguous = left.len() > 1
                    || right.len() > 1
                    || left.iter().any(|i| !pairable_cell(&before.cells[*i]))
                    || right.iter().any(|i| !pairable_cell(&after.cells[*i]));
                if ambiguous {
                    result.status = merge_status(result.status, NotebookStatus::Partial);
                    result.diagnostics.push(diagnostic(SymbolDiagnosticCode::AmbiguousCellIdentity,
                        right.first().or(left.first()).copied(), "ambiguous/occurrence-sensitive cell identity: no symbol pairing; before/after inventory retained"));
                    // Equal ordered normalized inventories establish no cell
                    // change, even when outputs or JSON representation differ.
                    // Keep identity uncertainty; do not pair symbols or infer
                    // identity through edits to this group.
                    if left
                        .iter()
                        .map(|i| &before.cells[*i].cell)
                        .eq(right.iter().map(|i| &after.cells[*i].cell))
                    {
                        continue;
                    }
                    for i in left {
                        cells.push(cell_event(
                            NotebookCellChangeKind::Deleted,
                            Some((before, &before.cells[i])),
                            None,
                        ));
                    }
                    for i in right {
                        cells.push(cell_event(
                            NotebookCellChangeKind::Added,
                            None,
                            Some((after, &after.cells[i])),
                        ));
                    }
                    continue;
                }
                match (left.first(), right.first()) {
                    (Some(l), Some(r)) => {
                        let l = &before.cells[*l];
                        let r = &after.cells[*r];
                        paired_cell_events(before, l, after, r, &mut cells);
                        if l.cell.language != r.cell.language
                            || l.cell.cell_type != r.cell.cell_type
                        {
                            one_sided_symbols(l, NotebookSymbolChangeKind::Deleted, &mut symbols);
                            one_sided_symbols(r, NotebookSymbolChangeKind::Added, &mut symbols);
                        } else if let Some(language) = r.cell.language {
                            let (changes, diagnostics) =
                                match_cell(before, l, after, r, language, rename)?;
                            symbols.extend(changes);
                            if !diagnostics.is_empty() {
                                result.status =
                                    merge_status(result.status, NotebookStatus::Partial);
                            }
                            result.diagnostics.extend(diagnostics);
                        }
                    }
                    (Some(l), None) => {
                        let l = &before.cells[*l];
                        cells.push(cell_event(
                            NotebookCellChangeKind::Deleted,
                            Some((before, l)),
                            None,
                        ));
                        one_sided_symbols(l, NotebookSymbolChangeKind::Deleted, &mut symbols);
                    }
                    (None, Some(r)) => {
                        let r = &after.cells[*r];
                        cells.push(cell_event(
                            NotebookCellChangeKind::Added,
                            None,
                            Some((after, r)),
                        ));
                        one_sided_symbols(r, NotebookSymbolChangeKind::Added, &mut symbols);
                    }
                    (None, None) => unreachable!(),
                }
            }
        }
        (Some(before), None) => {
            documents.push(document_event(
                NotebookDocumentChangeKind::Deleted,
                Some(before),
                None,
            ));
            for c in &before.cells {
                cells.push(cell_event(
                    NotebookCellChangeKind::Deleted,
                    Some((before, c)),
                    None,
                ));
                one_sided_symbols(c, NotebookSymbolChangeKind::Deleted, &mut symbols);
            }
        }
        (None, Some(after)) => {
            documents.push(document_event(
                NotebookDocumentChangeKind::Added,
                None,
                Some(after),
            ));
            for c in &after.cells {
                cells.push(cell_event(
                    NotebookCellChangeKind::Added,
                    None,
                    Some((after, c)),
                ));
                one_sided_symbols(c, NotebookSymbolChangeKind::Added, &mut symbols);
            }
        }
        (None, None) => unreachable!(),
    }
    // The ordinary matcher drains a HashMap. Give the new public contract stable
    // ordering without changing the ordinary wrapper's observable behavior.
    symbols.sort_by(|a, b| {
        let a = a.after.as_ref().or(a.before.as_ref()).unwrap();
        let b = b.after.as_ref().or(b.before.as_ref()).unwrap();
        a.source_ref
            .cell_ordinal
            .cmp(&b.source_ref.cell_ordinal)
            .then(a.symbol.byte_range.cmp(&b.symbol.byte_range))
            .then(a.id.cmp(&b.id))
    });
    result.symbol_changes = symbols;
    result.cell_changes = cells;
    result.document_changes = documents;
    Ok(result)
}

type SymbolIdentity<'a> = (&'a str, &'a str, Option<&'a str>);
fn identity(s: &NotebookSymbol) -> SymbolIdentity<'_> {
    (
        &s.symbol.entity_name,
        &s.symbol.symbol_kind,
        s.symbol.enclosing_scope.as_deref(),
    )
}
fn duplicate_symbol_identities(symbols: &[NotebookSymbol]) -> bool {
    let mut seen = HashSet::new();
    symbols.iter().any(|s| !seen.insert(identity(s)))
}
fn pairable_cell(c: &NotebookCellSymbols) -> bool {
    c.cell.identity_quality != CellIdentityQuality::Ambiguous
        && !c
            .cell
            .diagnostics
            .iter()
            .any(|d| d.code == NotebookDiagnosticCode::RepeatedAnonymousSource)
}

fn match_cell(
    before: &NotebookSymbolDocument,
    left: &NotebookCellSymbols,
    after: &NotebookSymbolDocument,
    right: &NotebookCellSymbols,
    language: Language,
    rename: bool,
) -> anyhow::Result<(Vec<NotebookSymbolChange>, Vec<SymbolDiagnostic>)> {
    let ordinal = Some(right.cell.ordinal);
    if duplicate_symbol_identities(&left.symbols) || duplicate_symbol_identities(&right.symbols) {
        return Ok((Vec::new(),vec![diagnostic(SymbolDiagnosticCode::AmbiguousSymbolIdentity, ordinal,
            "same-cell matching keys are ambiguous; matching skipped, complete symbol inventory retained")]));
    }
    let left_path: GitPath = canonical_cell_path(&before.path, &left.cell.id)
        .as_str()
        .into();
    let right_path: GitPath = canonical_cell_path(&after.path, &right.cell.id)
        .as_str()
        .into();
    // Retain FULL pre-match symbols by the exact snapshot keys that the matcher
    // uses. In particular a rename only carries the previous SnapshotKey.
    let Some(before_map) = snapshot_map(&left_path, &left.symbols) else {
        return Ok((
            vec![],
            vec![diagnostic(
                SymbolDiagnosticCode::SnapshotIdentityCollision,
                ordinal,
                "pre-match snapshot ID collision; matching skipped",
            )],
        ));
    };
    let Some(after_map) = snapshot_map(&right_path, &right.symbols) else {
        return Ok((
            vec![],
            vec![diagnostic(
                SymbolDiagnosticCode::SnapshotIdentityCollision,
                ordinal,
                "post-match snapshot ID collision; matching skipped",
            )],
        ));
    };
    let left_by_identity: HashMap<_, _> = left.symbols.iter().map(|s| (identity(s), s)).collect();
    let mut modified_before = HashMap::new();
    let mut unchanged_left = HashSet::new();
    let mut unchanged_right = HashSet::new();
    for r in &right.symbols {
        if let Some(l) = left_by_identity.get(&identity(r)) {
            let lk = snapshot_from(MATCH_CONTEXT, &left_path, &l.symbol).key;
            let rk = snapshot_from(MATCH_CONTEXT, &right_path, &r.symbol).key;
            // This is byte equality of the ORIGINAL slices, not a semantic
            // equivalence claim. Offset-only changes remain visible as cell
            // events and the returned after inventory has current refs.
            if !rename && original_slice(left, l) == original_slice(right, r) {
                unchanged_left.insert(lk);
                unchanged_right.insert(rk);
            } else {
                modified_before.insert(rk, lk);
            }
        }
    }
    let a: Vec<_> = left
        .symbols
        .iter()
        .filter(|s| {
            !unchanged_left.contains(&snapshot_from(MATCH_CONTEXT, &left_path, &s.symbol).key)
        })
        .map(|s| s.symbol.clone())
        .collect();
    let b: Vec<_> = right
        .symbols
        .iter()
        .filter(|s| {
            !unchanged_right.contains(&snapshot_from(MATCH_CONTEXT, &right_path, &s.symbol).key)
        })
        .map(|s| s.symbol.clone())
        .collect();
    let change = FileChange {
        path: right_path,
        kind: if rename {
            FileChangeKind::Renamed { from: left_path }
        } else {
            FileChangeKind::Modified
        },
        parent_sha: None,
    };
    let (changes, diagnostics) =
        symbol_changes_from_extracted_with_language(MATCH_CONTEXT, &change, language, &a, &b);
    let diagnostics = diagnostics
        .into_iter()
        .map(|message| SymbolDiagnostic {
            code: SymbolDiagnosticCode::AmbiguousRename,
            side: None,
            cell_ordinal: ordinal,
            byte_range: None,
            message,
        })
        .collect();
    let mut out = Vec::new();
    for c in changes {
        let key = &c.snapshot.key;
        let (kind, before_key, after_key) = match &c.change_kind {
            ChangeKind::Added => (NotebookSymbolChangeKind::Added, None, Some(key)),
            ChangeKind::Deleted => (NotebookSymbolChangeKind::Deleted, Some(key), None),
            ChangeKind::Modified => (
                NotebookSymbolChangeKind::Modified,
                Some(
                    modified_before
                        .get(key)
                        .ok_or_else(|| anyhow::anyhow!("matcher lost direct before identity"))?,
                ),
                Some(key),
            ),
            ChangeKind::RenamedFrom(RenamePrev::Symbol(previous)) => {
                (NotebookSymbolChangeKind::Renamed, Some(previous), Some(key))
            }
            ChangeKind::RenamedFrom(RenamePrev::File(_)) => {
                anyhow::bail!("inner symbol matcher returned file-level rename")
            }
        };
        let before = before_key
            .map(|k| {
                before_map
                    .get(k)
                    .cloned()
                    .ok_or_else(|| anyhow::anyhow!("matcher before snapshot missing"))
            })
            .transpose()?;
        let after = after_key
            .map(|k| {
                after_map
                    .get(k)
                    .cloned()
                    .ok_or_else(|| anyhow::anyhow!("matcher after snapshot missing"))
            })
            .transpose()?;
        out.push(NotebookSymbolChange {
            kind,
            before,
            after,
        });
    }
    Ok((out, diagnostics))
}

// An internal matcher context, never a caller snapshot/repository assertion.
const MATCH_CONTEXT: &str = "notebook-symbol-match-v1";
fn snapshot_map(
    path: &GitPath,
    symbols: &[NotebookSymbol],
) -> Option<HashMap<SnapshotKey, NotebookSymbol>> {
    let mut map = HashMap::new();
    for symbol in symbols {
        match map.entry(snapshot_from(MATCH_CONTEXT, path, &symbol.symbol).key) {
            std::collections::hash_map::Entry::Vacant(e) => {
                e.insert(symbol.clone());
            }
            std::collections::hash_map::Entry::Occupied(_) => return None,
        }
    }
    Some(map)
}
fn original_slice<'a>(cell: &'a NotebookCellSymbols, symbol: &NotebookSymbol) -> &'a str {
    let [a, b] = symbol.symbol.byte_range;
    &cell
        .cell
        .source
        .as_ref()
        .expect("symbols require valid source")
        .original[a..b]
}
fn one_sided_symbols(
    cell: &NotebookCellSymbols,
    kind: NotebookSymbolChangeKind,
    out: &mut Vec<NotebookSymbolChange>,
) {
    for s in &cell.symbols {
        out.push(NotebookSymbolChange {
            kind,
            before: (kind == NotebookSymbolChangeKind::Deleted).then(|| s.clone()),
            after: (kind == NotebookSymbolChangeKind::Added).then(|| s.clone()),
        });
    }
}
fn paired_cell_events(
    before: &NotebookSymbolDocument,
    l: &NotebookCellSymbols,
    after: &NotebookSymbolDocument,
    r: &NotebookCellSymbols,
    out: &mut Vec<NotebookCellChange>,
) {
    let a = &l.cell;
    let b = &r.cell;
    let directives = |c: &NormalizedCell| {
        c.source
            .as_ref()
            .map(|p| {
                p.directives
                    .iter()
                    .map(|d| d.text.clone())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
    };
    for (changed, kind) in [
        (a.ordinal != b.ordinal, NotebookCellChangeKind::Moved),
        (
            a.cell_type != b.cell_type,
            NotebookCellChangeKind::TypeChanged,
        ),
        (
            a.language != b.language,
            NotebookCellChangeKind::LanguageChanged,
        ),
        (
            a.source.as_ref().map(|p| &p.original) != b.source.as_ref().map(|p| &p.original),
            NotebookCellChangeKind::SourceChanged,
        ),
        (
            a.metadata != b.metadata,
            NotebookCellChangeKind::MetadataChanged,
        ),
        (
            directives(a) != directives(b),
            NotebookCellChangeKind::DependencyDirectivesChanged,
        ),
    ] {
        if changed {
            out.push(cell_event(kind, Some((before, l)), Some((after, r))));
        }
    }
}
fn cell_ref(doc: &NotebookSymbolDocument, c: &NotebookCellSymbols) -> CellRef {
    CellRef {
        notebook_path: doc.path.clone(),
        notebook_source_sha256: doc.source_sha256.clone(),
        cell_id: c.cell.id.clone(),
        cell_ordinal: c.cell.ordinal,
        cell_source_sha256: c.cell.source.as_ref().map(|p| p.original_sha256.clone()),
        language: c.cell.language,
        cell_type: c.cell.cell_type.clone(),
    }
}
fn cell_event(
    kind: NotebookCellChangeKind,
    before: Option<(&NotebookSymbolDocument, &NotebookCellSymbols)>,
    after: Option<(&NotebookSymbolDocument, &NotebookCellSymbols)>,
) -> NotebookCellChange {
    NotebookCellChange {
        kind,
        before: before.map(|(d, c)| cell_ref(d, c)),
        after: after.map(|(d, c)| cell_ref(d, c)),
    }
}
fn document_event(
    kind: NotebookDocumentChangeKind,
    before: Option<&NotebookSymbolDocument>,
    after: Option<&NotebookSymbolDocument>,
) -> NotebookDocumentChange {
    NotebookDocumentChange {
        kind,
        before_path: before.map(|d| d.path.clone()),
        after_path: after.map(|d| d.path.clone()),
        before_metadata: before.map(|d| d.metadata.clone()),
        after_metadata: after.map(|d| d.metadata.clone()),
    }
}
fn normalization_diagnostic(d: NotebookDiagnostic) -> SymbolDiagnostic {
    SymbolDiagnostic {
        code: SymbolDiagnosticCode::Normalization(d.code),
        side: None,
        cell_ordinal: d.cell_ordinal,
        byte_range: d.byte_range,
        message: d.message,
    }
}
fn diagnostic(
    code: SymbolDiagnosticCode,
    cell_ordinal: Option<usize>,
    message: &str,
) -> SymbolDiagnostic {
    SymbolDiagnostic {
        code,
        side: None,
        cell_ordinal,
        byte_range: None,
        message: message.into(),
    }
}
fn merge_status(a: NotebookStatus, b: NotebookStatus) -> NotebookStatus {
    match (a, b) {
        (NotebookStatus::Invalid, _) | (_, NotebookStatus::Invalid) => NotebookStatus::Invalid,
        (NotebookStatus::Partial, _) | (_, NotebookStatus::Partial) => NotebookStatus::Partial,
        _ => NotebookStatus::Complete,
    }
}
fn valid_path(path: &str) -> bool {
    !path.trim().is_empty() && !path.contains('\0')
}
fn canonical_cell_path(path: &str, id: &str) -> String {
    // Percent is escaped too. Every byte outside this alphabet has one uppercase
    // escape, so an embedded delimiter cannot cross the notebook/cell boundary.
    fn escape(value: &str) -> String {
        let mut out = String::new();
        for b in value.bytes() {
            if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~' | b'/') {
                out.push(b as char);
            } else {
                use std::fmt::Write;
                write!(&mut out, "%{b:02X}").unwrap();
            }
        }
        out
    }
    format!("{}#cell:{}", escape(path), escape(id))
}

#[cfg(test)]
#[path = "notebook_symbols_tests.rs"]
mod tests;
