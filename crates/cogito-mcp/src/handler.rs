//! Minimal `rmcp::ClientHandler` impl.
//!
//! rmcp requires a `ClientHandler` to spawn a service; we don't need
//! the full surface (no elicitation UI in v0.1), so this is a thin
//! shell that:
//!
//! - Rejects elicitation requests (server tries to ask the user via
//!   client) with `rmcp::model::ErrorData::method_not_found`.
//! - Forwards logging / progress / cancellation / `resource_updated`
//!   notifications to `tracing`.
//!
//! See ADR-0018 for the elicitation rationale (out of scope for v0.1).

use rmcp::ClientHandler;
use rmcp::RoleClient;
use rmcp::model::{
    CancelledNotificationParam, ClientInfo, CreateElicitationRequestParams,
    CreateElicitationResult, ElicitationCreateRequestMethod, ErrorData,
    LoggingMessageNotificationParam, ProgressNotificationParam, ResourceUpdatedNotificationParam,
};
use rmcp::service::{NotificationContext, RequestContext};
use tracing::{debug, info, warn};

/// Identifies the server this handler belongs to, so trace fields
/// can group messages by origin.
#[derive(Clone)]
pub(crate) struct MinimalClientHandler {
    server_name: String,
    client_info: ClientInfo,
}

impl MinimalClientHandler {
    pub(crate) fn new(server_name: String, client_info: ClientInfo) -> Self {
        Self {
            server_name,
            client_info,
        }
    }
}

impl ClientHandler for MinimalClientHandler {
    fn get_info(&self) -> ClientInfo {
        self.client_info.clone()
    }

    async fn create_elicitation(
        &self,
        _request: CreateElicitationRequestParams,
        _context: RequestContext<RoleClient>,
    ) -> Result<CreateElicitationResult, ErrorData> {
        warn!(
            mcp.server = %self.server_name,
            "MCP server requested elicitation; cogito v0.1 does not support it"
        );
        Err(ErrorData::method_not_found::<ElicitationCreateRequestMethod>())
    }

    async fn on_logging_message(
        &self,
        params: LoggingMessageNotificationParam,
        _context: NotificationContext<RoleClient>,
    ) {
        // Forward MCP log levels to tracing levels (best-effort mapping).
        let level_str = format!("{:?}", params.level);
        info!(
            mcp.server = %self.server_name,
            mcp.log_level = %level_str,
            mcp.logger = ?params.logger,
            "{}",
            serde_json::to_string(&params.data).unwrap_or_default()
        );
    }

    async fn on_progress(
        &self,
        params: ProgressNotificationParam,
        _context: NotificationContext<RoleClient>,
    ) {
        debug!(
            mcp.server = %self.server_name,
            mcp.progress_token = ?params.progress_token,
            mcp.progress = params.progress,
            mcp.total = ?params.total,
            "MCP progress notification"
        );
    }

    async fn on_cancelled(
        &self,
        params: CancelledNotificationParam,
        _context: NotificationContext<RoleClient>,
    ) {
        info!(
            mcp.server = %self.server_name,
            mcp.request_id = %params.request_id,
            mcp.reason = ?params.reason,
            "MCP server cancelled a request"
        );
    }

    async fn on_resource_updated(
        &self,
        params: ResourceUpdatedNotificationParam,
        _context: NotificationContext<RoleClient>,
    ) {
        info!(
            mcp.server = %self.server_name,
            mcp.resource_uri = %params.uri,
            "MCP server reported resource updated"
        );
    }
}
