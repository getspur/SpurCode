pub(crate) mod code_graph;
pub(crate) mod delegation;
pub(crate) mod plan;
pub(crate) mod plan_execute;

use super::McpCallbackServer;
use super::*;

impl McpCallbackServer {
    pub(crate) fn rmcp_tools(&self) -> Vec<Tool> {
        self.tool_registry
            .list_tools()
            .into_iter()
            .map(|def| Tool::new(def.name, def.description, rmcp_object(def.input_schema)))
            .collect()
    }

    pub(crate) fn rmcp_tool(&self, name: &str) -> Option<Tool> {
        self.rmcp_tools().into_iter().find(|tool| tool.name == name)
    }

    pub(crate) fn call_tool_result_from_legacy_response(
        response: JsonRpcResponse,
        tool_name: &str,
    ) -> Result<CallToolResult, McpError> {
        spur_mcp::json_rpc_to_call_tool_result(response, tool_name)
    }

    // ─── Tool call dispatcher ─────────────────────────────────────────

    pub(crate) async fn handle_tool_call(&self, id: Value, params: Value) -> JsonRpcResponse {
        self.handle_tool_call_with_outcome(id, params).await.0
    }

    pub(crate) async fn handle_tool_call_with_outcome(
        &self,
        id: Value,
        params: Value,
    ) -> (JsonRpcResponse, spur_telemetry::tier1_events::Outcome) {
        let tool_name = match params.get("name").and_then(|v| v.as_str()) {
            Some(name) => name.to_string(),
            None => {
                return (
                    JsonRpcResponse::invalid_params(id, "Missing 'name' in params"),
                    spur_telemetry::tier1_events::Outcome::Error,
                );
            }
        };

        let arguments = params
            .get("arguments")
            .cloned()
            .unwrap_or_else(|| json!({}));

        debug!(tool = %tool_name, "Handling tool call");

        if tokio::time::timeout(BRAIN_SESSION_BIND_TIMEOUT, self.brain_session_id_ready())
            .await
            .is_err()
        {
            let outcome = spur_telemetry::tier1_events::Outcome::Timeout;
            let response =
                JsonRpcResponse::internal_error(id, "server not yet bound to brain session");
            Self::emit_mcp_tool_called("spur-mcp", &tool_name, outcome);
            return (response, outcome);
        }

        let ctx = spur_mcp::registry::ToolCallContext::new(
            spur_mcp::registry::ServerKind::Brain,
            spur_mcp::registry::ToolAuthority::Brain,
            self.brain_session_id.get(),
            Some(&id),
        );
        let canonical_tool_name = match self.tool_registry.canonical_name_for_call(&tool_name) {
            Ok(name) => name.to_string(),
            Err(error) => {
                let outcome = spur_telemetry::tier1_events::Outcome::Error;
                let response = JsonRpcResponse::mcp_error(id, error);
                Self::emit_mcp_tool_called("spur-mcp", &tool_name, outcome);
                return (response, outcome);
            }
        };
        let response = if crate::mcp::plan::is_plan_tool(&canonical_tool_name) {
            crate::mcp::plan::PlanMcpModule::new(crate::mcp::plan::PlanMcpDeps::from_server(self))
                .call_with_server(self, ctx, &canonical_tool_name, arguments)
                .await
        } else if crate::mcp::catalog::is_server_owned_tool(&canonical_tool_name) {
            self.handle_registered_tool_call(ctx, &canonical_tool_name, arguments)
                .await
        } else {
            self.tool_registry
                .call_json_tool(ctx, &tool_name, arguments)
                .await
        };
        let outcome = Self::telemetry_outcome_from_json_rpc_response(&response);
        Self::emit_mcp_tool_called("spur-mcp", &canonical_tool_name, outcome);

        (response, outcome)
    }

    pub(crate) async fn handle_registered_tool_call(
        &self,
        ctx: spur_mcp::registry::ToolCallContext<'_>,
        tool_name: &str,
        arguments: Value,
    ) -> JsonRpcResponse {
        let id = ctx.request_id_value();

        match tool_name {
            "code_resolve" => self.handle_code_resolve(id, arguments).await,
            // `code_search` is the legacy alias for `code_symbol_search`.
            "code_symbol_search" | "code_search" => self.handle_code_search(id, arguments).await,
            "code_file_symbols" => self.handle_code_file_symbols(id, arguments).await,
            "code_symbol_info" => self.handle_code_symbol_info(id, arguments).await,
            "code_read_symbol" => self.handle_code_read_symbol(id, arguments).await,
            "code_callers" => self.handle_code_callers(id, arguments).await,
            "code_callees" => self.handle_code_callees(id, arguments).await,
            "code_subgraph" => self.handle_code_subgraph(id, arguments).await,
            "code_symbol_history" => self.handle_code_symbol_history(id, arguments).await,
            tool if crate::mcp::catalog::is_pm_tool(tool) => {
                self.handle_pm_tool(id, tool, arguments).await
            }
            tool if crate::mcp::catalog::is_analyst_tool(tool) => {
                self.handle_analyst_tool(id, tool, arguments).await
            }
            _ => JsonRpcResponse::error(id, -32601, format!("Unknown tool: {tool_name}")),
        }
    }

    async fn handle_pm_tool(&self, id: Value, tool_name: &str, args: Value) -> JsonRpcResponse {
        let event_sink = self.event_sink.clone();
        let on_issue_created = event_sink.map(|sink| {
            Arc::new(move |event: spur_pm::mcp::IssueCreatedEvent| {
                let issue = crate::mcp::catalog::issue_to_summary_event(&event.issue, event.source);
                if sink
                    .try_emit(spur_acp::SpurEventBody::IssueCreated { issue })
                    .is_err()
                {
                    tracing::warn!(
                        issue_id = %event.issue.id,
                        "dropping IssueCreated event because broadcast sink is full"
                    );
                }
            }) as Arc<dyn Fn(spur_pm::mcp::IssueCreatedEvent) + Send + Sync>
        });

        let module = spur_pm::mcp::PmMcpModule::new(spur_pm::mcp::PmMcpDeps {
            pm_service: self.pm_service.clone(),
            on_issue_created,
        });
        match module.call(tool_name, args).await {
            Ok(result) => JsonRpcResponse::success(id, result),
            Err(spur_pm::mcp::McpHandlerError::InvalidParams(message)) => {
                JsonRpcResponse::invalid_params(id, message)
            }
            Err(spur_pm::mcp::McpHandlerError::NotFound(message)) => {
                JsonRpcResponse::error(id, -32004, message)
            }
            Err(spur_pm::mcp::McpHandlerError::Unauthorized(message)) => {
                JsonRpcResponse::error(id, -32001, message)
            }
            Err(spur_pm::mcp::McpHandlerError::UpstreamPm(message)) => {
                JsonRpcResponse::internal_error(id, format!("{tool_name} failed: {message}"))
            }
            Err(spur_pm::mcp::McpHandlerError::Internal(message)) => {
                JsonRpcResponse::internal_error(id, message)
            }
        }
    }

    async fn handle_analyst_tool(
        &self,
        id: Value,
        tool_name: &str,
        args: Value,
    ) -> JsonRpcResponse {
        let module = spur_analyst::mcp::AnalystMcpModule::with_local_projects(
            self.local_projects.resolver(),
        )
        .with_embedding_runtime(self.embedding_runtime.clone());
        let dispatch = async move { module.dispatch(tool_name, args).await };
        let result = match self.repo_root.clone() {
            Some(root) => spur_graph::mcp::with_worktree_root_for_request(root, dispatch).await,
            None => dispatch.await,
        };
        match result {
            Ok(body) => {
                let text = serde_json::to_string_pretty(&body).unwrap_or_else(|_| body.to_string());
                JsonRpcResponse::success(
                    id,
                    json!({ "content": [{ "type": "text", "text": text }] }),
                )
            }
            Err(spur_analyst::mcp::McpHandlerError::InvalidParams(message)) => {
                JsonRpcResponse::invalid_params(id, message)
            }
            Err(spur_analyst::mcp::McpHandlerError::NotFound(message)) => {
                JsonRpcResponse::error(id, -32004, message)
            }
            Err(error) => JsonRpcResponse::internal_error(id, error.to_string()),
        }
    }

    pub(crate) fn telemetry_mcp_server_name(
        name: &str,
    ) -> spur_telemetry::tier2_events::McpServerName {
        match name {
            "github" => spur_telemetry::tier2_events::McpServerName::Github,
            "posthog" => spur_telemetry::tier2_events::McpServerName::Posthog,
            "spur-mcp" => spur_telemetry::tier2_events::McpServerName::SpurMcp,
            "stitch" => spur_telemetry::tier2_events::McpServerName::Stitch,
            "playwright" => spur_telemetry::tier2_events::McpServerName::Playwright,
            "context7" => spur_telemetry::tier2_events::McpServerName::Context7,
            "firebase" => spur_telemetry::tier2_events::McpServerName::Firebase,
            "sequential-thinking" => {
                spur_telemetry::tier2_events::McpServerName::SequentialThinking
            }
            _ => spur_telemetry::tier2_events::McpServerName::Custom(
                spur_telemetry::tier2_events::HashedShort::from_sha256_prefix(name),
            ),
        }
    }

    pub(crate) fn telemetry_mcp_tool_name(name: &str) -> spur_telemetry::tier2_events::McpToolName {
        match name {
            "submit_plan" => spur_telemetry::tier2_events::McpToolName::SubmitPlan,
            "dispatch_task" => spur_telemetry::tier2_events::McpToolName::DispatchTask,
            "review_task" => spur_telemetry::tier2_events::McpToolName::ReviewTask,
            "get_task_diff" => spur_telemetry::tier2_events::McpToolName::GetTaskDiff,
            "list_tools" => spur_telemetry::tier2_events::McpToolName::ListTools,
            _ => spur_telemetry::tier2_events::McpToolName::Custom(
                spur_telemetry::tier2_events::HashedShort::from_sha256_prefix(name),
            ),
        }
    }

    pub(crate) fn telemetry_outcome_from_json_rpc_response(
        response: &JsonRpcResponse,
    ) -> spur_telemetry::tier1_events::Outcome {
        if response.error.is_some() {
            spur_telemetry::tier1_events::Outcome::Error
        } else {
            spur_telemetry::tier1_events::Outcome::Ok
        }
    }

    pub(crate) fn emit_mcp_request_duration(
        duration: std::time::Duration,
        outcome: spur_telemetry::tier1_events::Outcome,
    ) {
        let duration_ms = duration.as_millis().min(u128::from(u64::MAX)) as u64;
        spur_telemetry::emit!(spur_telemetry::tier1_events::McpRequestDuration {
            duration_ms,
            outcome,
        });
    }

    pub(crate) fn emit_mcp_tool_called(
        server_name: &str,
        tool_name: &str,
        outcome: spur_telemetry::tier1_events::Outcome,
    ) {
        spur_telemetry::emit!(spur_telemetry::tier2_events::McpToolCalled {
            server_name: Self::telemetry_mcp_server_name(server_name),
            tool_name: Self::telemetry_mcp_tool_name(tool_name),
            outcome,
        });
    }
}
