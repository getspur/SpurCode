use serde_json::Value;

use crate::embedding::OwnedEmbeddingRuntime;
use crate::mcp::McpHandlerError;
use crate::pack::{service, KnowledgeContextPackRequest, KnowledgeContextPackV2Request};

pub(super) async fn knowledge_context_pack_with_runtime(
    args: &Value,
    runtime: &OwnedEmbeddingRuntime,
) -> Result<Value, McpHandlerError> {
    let request = KnowledgeContextPackRequest::parse(args)?;
    service::knowledge_context_pack_with_runtime(request, runtime).await
}

pub async fn knowledge_context_pack_2_with_runtime(
    args: &Value,
    runtime: &OwnedEmbeddingRuntime,
) -> Result<Value, McpHandlerError> {
    let request = KnowledgeContextPackV2Request::parse(args)?;
    service::knowledge_context_pack_2_with_runtime(request, runtime).await
}
