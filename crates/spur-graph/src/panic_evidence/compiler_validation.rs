//! Compiler/source correspondence without discharge of semantic obligations.
//! All raw observations survive, including macro-definition and external spans.
use anyhow::{ensure, Context as _};
use serde::{Deserialize, Serialize};
use tree_sitter::Parser;

use super::*;

pub const PANIC_COMPILER_VALIDATION_SCHEMA_VERSION: &str = "panic-compiler-validation-v1";

/// Independently retained producer declaration joining the capture identity to
/// the complete semantic build (including edition, resolved cfg and check modes).
/// The legacy producer format does not export those resolved settings. A caller
/// must obtain this binding from its trusted capture procedure, not manufacture
/// it from untrusted facts. This API validates the join, not that a compiler ran.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PanicCompilerConfigBinding {
    pub capture_build_id: String,
    pub semantic_build_hash: String,
}

pub struct PanicCompilerValidationRequest<'a> {
    pub import: CgaImportRequest<'a>,
    pub compiler: &'a CompilerEvidenceSet,
    pub calls: &'a RustCallSiteEvidenceSet,
    pub facts: &'a PanicSemanticFacts,
    pub expected_build: &'a PanicBuildConfig,
    pub config_binding: &'a PanicCompilerConfigBinding,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PanicCompilerCompleteness {
    Unestablished,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PanicCompilerLimitation {
    SourceCorrespondenceOnly,
    ReceiverTypeUnavailable,
    AssertReasonUnavailable,
    ExpansionInvocationUnavailable,
    GenericVirtualProducerTrust,
    ConfigurationDeclarationTrust,
    CaptureProcedureTrust,
    SyntheticCapture,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PanicCompilerLocationKind {
    BodyOwned,
    MacroDefinition,
    Unmapped,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PanicCompilerInstance {
    /// Index into compiler.raw_graph.nodes; the opaque name/location stay there.
    pub node_id: u32,
    pub body_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PanicCompilerCorrelation {
    /// Index into compiler.observations, which retains from_node and edge_index.
    pub observation_index: usize,
    pub location_kind: PanicCompilerLocationKind,
    pub body_id: Option<String>,
    /// Exact spans only, with independently established innermost body ownership.
    /// These lists do not resolve callees or assert feasible execution.
    pub call_site_ids: Vec<String>,
    pub semantic_site_ids: Vec<String>,
    pub implicit_operation_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PanicCompilerValidation {
    pub schema_version: String,
    pub semantic_build_hash: String,
    pub config_binding: PanicCompilerConfigBinding,
    pub compiler: CompilerEvidenceSet,
    pub instances: Vec<PanicCompilerInstance>,
    pub correlations: Vec<PanicCompilerCorrelation>,
    /// Disjoint raw categories. Called is a compiler observation, not a witness
    /// that a path is feasible; virtual and address observations stay separate.
    pub called_observations: Vec<usize>,
    pub virtual_observations: Vec<usize>,
    pub address_observations: Vec<usize>,
    pub implicit_assert_observations: Vec<usize>,
    pub async_panic_observations: Vec<usize>,
    pub retained_unknown_obligations: Vec<PanicUnknownObligation>,
    /// v1 cannot discharge any obligation: it has no expansion stack, receiver
    /// types, path constraints, assert reasons or producer correctness proof.
    pub cleared_obligations: Vec<String>,
    pub semantic_completeness: PanicCompilerCompleteness,
    pub limitations: Vec<PanicCompilerLimitation>,
}

impl PanicCompilerValidation {
    /// Reconstruct from trusted inputs. Reject forged raw references, spatial
    /// owners, categories, omissions and claims that uncertainty was cleared.
    pub fn validate(&self, request: PanicCompilerValidationRequest<'_>) -> anyhow::Result<()> {
        ensure!(
            self == &correlate_panic_compiler(request)?,
            "compiler validation differs from canonical correlation"
        );
        Ok(())
    }
}

pub fn correlate_panic_compiler(
    request: PanicCompilerValidationRequest<'_>,
) -> anyhow::Result<PanicCompilerValidation> {
    let r = &request.import;
    // Use the legacy importer unchanged, including pinned identity, raw checksum,
    // source inventory, every edge reference and physical coordinate validation.
    let compiler = import_cga(CgaImportRequest {
        identity: r.identity,
        expected_build_id: r.expected_build_id,
        raw_export: r.raw_export,
        inputs: r.inputs,
        graph: r.graph,
        sources: r.sources,
        method_sites: r.method_sites,
    })?;
    ensure!(
        &compiler == request.compiler,
        "compiler evidence differs from canonical import"
    );
    request.calls.validate(r.graph, r.sources)?;
    request
        .facts
        .validate_against(r.graph, r.sources, request.calls)?;
    request
        .facts
        .validate_canonical(r.graph, r.sources, request.expected_build)?;
    let build_hash = request.expected_build.content_hash()?;
    ensure!(
        request.config_binding.capture_build_id == compiler.build_id
            && request.config_binding.semantic_build_hash == build_hash,
        "compiler/semantic configuration binding mismatch"
    );
    let build = request.expected_build;
    let mut features = compiler.identity.features.clone();
    features.sort();
    ensure!(
        build.target == compiler.identity.target
            && build.rustc_verbose == compiler.identity.rustc_verbose
            && build.codegen_flags == compiler.identity.compiler_flags
            && build.features == features,
        "compiler and semantic build settings disagree"
    );
    validate_check_flags(build)?;

    let body_locations = cga::cga_body_locations(&compiler.raw_graph, r)?;
    let instances: Vec<_> = body_locations
        .iter()
        .enumerate()
        .map(|(id, location)| {
            let mut matching = request.calls.bodies.iter().filter(|b| {
                location.as_ref().is_some_and(|l| {
                    l.file_path.as_ref() == Some(&b.file_path) && l.byte_range == Some(b.byte_range)
                })
            });
            let first = matching.next();
            let body_id = if matching.next().is_none() {
                first.map(|b| b.body_id.clone())
            } else {
                None
            };
            PanicCompilerInstance {
                node_id: id as u32,
                body_id,
            }
        })
        .collect();
    let macros = macro_definitions(r.sources)?;
    let mut report = PanicCompilerValidation {
        schema_version: PANIC_COMPILER_VALIDATION_SCHEMA_VERSION.into(),
        semantic_build_hash: build_hash,
        config_binding: request.config_binding.clone(),
        compiler,
        instances,
        correlations: vec![],
        called_observations: vec![],
        virtual_observations: vec![],
        address_observations: vec![],
        implicit_assert_observations: vec![],
        async_panic_observations: vec![],
        retained_unknown_obligations: request.facts.unknown_obligations.clone(),
        cleared_obligations: vec![],
        semantic_completeness: PanicCompilerCompleteness::Unestablished,
        limitations: vec![
            PanicCompilerLimitation::SourceCorrespondenceOnly,
            PanicCompilerLimitation::ReceiverTypeUnavailable,
            PanicCompilerLimitation::AssertReasonUnavailable,
            PanicCompilerLimitation::ExpansionInvocationUnavailable,
            PanicCompilerLimitation::GenericVirtualProducerTrust,
            PanicCompilerLimitation::ConfigurationDeclarationTrust,
            PanicCompilerLimitation::CaptureProcedureTrust,
        ],
    };
    if report.compiler.identity.provenance == ExportProvenance::SyntheticFixture {
        report
            .limitations
            .push(PanicCompilerLimitation::SyntheticCapture);
    }
    for (i, obs) in report.compiler.observations.iter().enumerate() {
        match obs.kind {
            CgaEdgeKind::Called(_) => report.called_observations.push(i),
            CgaEdgeKind::VTableCalled(_) => report.virtual_observations.push(i),
            CgaEdgeKind::FnPtrCast(_) => report.address_observations.push(i),
            CgaEdgeKind::BuiltinAssert => report.implicit_assert_observations.push(i),
            CgaEdgeKind::AsyncPanic => report.async_panic_observations.push(i),
        }
        let path = obs.converted_location.file_path.as_deref();
        let range = obs.converted_location.byte_range;
        let macro_definition = macros
            .iter()
            .any(|(p, span)| path == Some(p.as_str()) && range.is_some_and(|r| contains(*span, r)));
        // A nested/deferred/constant body must not be assigned to its ancestor.
        let spatial_body = request
            .calls
            .bodies
            .iter()
            .filter(|b| {
                path == Some(b.file_path.as_str())
                    && range.is_some_and(|r| contains(b.byte_range, r))
            })
            .min_by_key(|b| b.byte_range[1] - b.byte_range[0]);
        let body_id = spatial_body
            .filter(|b| {
                !macro_definition
                    && report.instances[obs.from_node as usize].body_id.as_deref()
                        == Some(&b.body_id)
            })
            .map(|b| b.body_id.clone());
        let mut c = PanicCompilerCorrelation {
            observation_index: i,
            location_kind: if macro_definition {
                PanicCompilerLocationKind::MacroDefinition
            } else if body_id.is_some() {
                PanicCompilerLocationKind::BodyOwned
            } else {
                PanicCompilerLocationKind::Unmapped
            },
            body_id,
            call_site_ids: vec![],
            semantic_site_ids: vec![],
            implicit_operation_ids: vec![],
        };
        if let Some(owner) = &c.body_id {
            let matches = |loc: &PanicFactLocation| {
                loc.body_id == *owner
                    && Some(loc.file_path.as_str()) == path
                    && Some(loc.byte_range) == range
            };
            if matches!(obs.kind, CgaEdgeKind::Called(_)) {
                c.call_site_ids = request
                    .calls
                    .sites
                    .iter()
                    .filter(|s| {
                        s.body_id == *owner
                            && Some(s.file_path.as_str()) == path
                            && Some(s.byte_range) == range
                    })
                    .map(|s| s.site_id.clone())
                    .collect();
                c.semantic_site_ids = request
                    .facts
                    .sites
                    .iter()
                    .filter(|s| matches(&s.context.location))
                    .map(|s| s.site_id.clone())
                    .collect();
            }
            if obs.kind == CgaEdgeKind::BuiltinAssert {
                if let Some(extraction) = &request.facts.extraction {
                    c.implicit_operation_ids = extraction
                        .operations
                        .iter()
                        .filter(|op| matches(&op.context.location))
                        .map(|op| op.operation_id.clone())
                        .collect();
                }
            }
        }
        report.correlations.push(c);
    }
    Ok(report)
}

fn contains(outer: [usize; 2], inner: [usize; 2]) -> bool {
    inner[0] < inner[1] && outer[0] <= inner[0] && inner[1] <= outer[1]
}

fn macro_definitions(
    sources: &[PanicEvidenceSource<'_>],
) -> anyhow::Result<Vec<(String, [usize; 2])>> {
    let mut parser = Parser::new();
    parser.set_language(&tree_sitter_rust::LANGUAGE.into())?;
    let mut spans = vec![];
    for s in sources {
        let tree = parser
            .parse(s.bytes, None)
            .context("macro definition parse unavailable")?;
        let mut pending = vec![tree.root_node()];
        while let Some(n) = pending.pop() {
            if n.kind() == "macro_definition" {
                spans.push((s.file_path.into(), [n.start_byte(), n.end_byte()]));
                continue;
            }
            if matches!(n.kind(), "macro_invocation" | "token_tree") {
                continue;
            }
            let mut cursor = n.walk();
            pending.extend(n.named_children(&mut cursor));
        }
    }
    Ok(spans)
}

/// Check explicit effective check flags when present. Absent flags are covered
/// only by the independently retained complete configuration declaration; the
/// profile/target defaults are deliberately not inferred here.
fn validate_check_flags(build: &PanicBuildConfig) -> anyhow::Result<()> {
    let mut options = std::collections::BTreeMap::new();
    let mut args = build.codegen_flags.iter();
    while let Some(arg) = args.next() {
        let option = if arg == "-C" || arg == "--codegen" {
            args.next().map(String::as_str)
        } else {
            arg.strip_prefix("-C")
                .or_else(|| arg.strip_prefix("--codegen="))
        };
        if let Some(option) = option {
            let (key, value) = option
                .split_once('=')
                .map(|(key, value)| (key, Some(value)))
                .unwrap_or((option, None));
            // rustc accepts underscore aliases and bare boolean options. Parse
            // each recognized occurrence, then retain the last canonical key.
            let key = match key {
                "overflow-checks" | "overflow_checks" => "overflow-checks",
                "debug-assertions" | "debug_assertions" => "debug-assertions",
                "panic" => {
                    if let Some(value) = value {
                        options.insert("panic", value);
                    }
                    continue;
                }
                _ => continue,
            };
            let value = match value {
                None | Some("on" | "yes" | "true" | "y") => "on",
                Some("off" | "no" | "false" | "n") => "off",
                _ => anyhow::bail!("unsupported explicit check flag"),
            };
            options.insert(key, value);
        }
    }
    for (key, expected) in [
        ("overflow-checks", build.overflow_checks),
        ("debug-assertions", build.debug_assertions),
    ] {
        if let Some(value) = options.get(key) {
            let actual = match *value {
                "on" | "yes" | "true" | "y" => true,
                "off" | "no" | "false" | "n" => false,
                _ => anyhow::bail!("unsupported explicit check flag"),
            };
            ensure!(
                actual == expected,
                "compiler {key} disagrees with semantic build"
            );
        }
    }
    if let Some(value) = options.get("panic") {
        ensure!(
            *value
                == match build.panic_strategy {
                    PanicStrategy::Unwind => "unwind",
                    PanicStrategy::Abort => "abort",
                },
            "compiler panic strategy disagrees with semantic build"
        );
    }
    Ok(())
}
