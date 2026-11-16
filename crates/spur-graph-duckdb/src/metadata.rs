//! Additive runtime contract. The extractor ID identifies source inputs; the
//! packaged artifact's SHA belongs to its packaging receipt, not this STRUCT.

use std::collections::BTreeSet;
use std::path::Path;

use duckdb_neo::connection::Context;
use duckdb_neo::data_chunk::VectorCollection;
use duckdb_neo::error::Error;
use duckdb_neo::extension::Extension;
use duckdb_neo::scalar::{ScalarCallbacks, ScalarFunctionBuilder};
use duckdb_neo::signature::{FunctionDocumentationBuilder, SignatureBuilder};
use duckdb_neo::types::List;
use duckdb_neo::vector::{RowWriter, Unknown, Vector};
use duckdb_neo::{AsDuckDB, DuckDBType, Result};
use spur_graph::extract::languages::{all_supported_extensions, Language};

#[derive(AsDuckDB)]
#[duckdb(writer)]
struct GraphInfo {
    extension_version: String,
    facts_schema_version: u32,
    duckdb_api_version: String,
    extractor_id: String,
    // Native Language variant names, matching the native catalog examples.
    supported_languages: List<String>,
    resolution_scope: String,
}

struct Metadata;

impl ScalarCallbacks for Metadata {
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
        let languages = all_supported_extensions()
            .into_iter()
            .map(|extension| {
                let path = format!("input.{extension}");
                Language::from_path(Path::new(&path))
                    .map(|language| format!("{language:?}"))
                    .ok_or_else(|| Error::internal("native registered extension has no language"))
            })
            .collect::<Result<BTreeSet<_>>>()?;
        let info = GraphInfo {
            extension_version: env!("CARGO_PKG_VERSION").to_owned(),
            facts_schema_version: 1,
            duckdb_api_version: "2.0.0".to_owned(),
            extractor_id: env!("SPUR_GRAPH_EXTRACTOR_ID").to_owned(),
            supported_languages: languages.into_iter().map(Some).collect(),
            resolution_scope: "single_file".to_owned(),
        };
        let count = vectors.row_count()?;
        let mut output = output.cast::<GraphInfo>()?;
        output.set_size(count)?;
        output
            .children_mut()
            .ok_or_else(|| Error::internal("spur_graph_info: result fields missing"))?
            .get_vector_at_mut::<List<String>>(4)?
            .set_child_size(count * info.supported_languages.0.len())?;
        let mut writer = output.row_writer()?;
        for index in 0..count {
            writer.set(index, &info)?;
        }
        Ok(())
    }
}

pub(crate) fn register(extension: &Extension) -> Result<()> {
    ScalarFunctionBuilder::new(
        "spur_graph_info",
        SignatureBuilder::new()
            .return_type(GraphInfo::logical_type(extension)?)
            .documentation(
                FunctionDocumentationBuilder::new()
                    .description("Versioned single-file facts contract; extractor_id hashes source inputs, not binary bytes; supported_languages uses native Language variant names")
                    .add_example("Inspect extraction contract", "SELECT spur_graph_info()")
                    .add_category("spur_graph"),
            ),
        Metadata,
    )
    .register(extension)
}
