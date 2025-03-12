use mcp_core::error::Error as McpError;
use mcp_core::schema::*;
use mcp_core::service::{PeerProxy, RoleServer, Service, ServiceError};
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct ServerHandlerService<H> {
    pub handler: H,
}

impl<H: Handler> ServerHandlerService<H> {
    pub fn new(handler: H) -> Self {
        Self { handler }
    }
}

impl<H: Handler> Service for ServerHandlerService<H> {
    type Role = RoleServer;

    async fn handle_request(
        &self,
        request: <Self::Role as mcp_core::service::ServiceRole>::PeerReq,
    ) -> Result<<Self::Role as mcp_core::service::ServiceRole>::Resp, mcp_core::service::ServiceError>
    {
        let result = match request {
            ClientRequest::PingRequest(_request) => {
                Ok(ServerResult::EmptyResult(EmptyResult::default()))
            }
            ClientRequest::InitializeRequest(request) => self
                .handler
                .initialize(request.params)
                .await
                .map(ServerResult::InitializeResult),
            ClientRequest::CompleteRequest(request) => self
                .handler
                .complete(request.params)
                .await
                .map(ServerResult::CompleteResult),
            ClientRequest::SetLevelRequest(request) => self
                .handler
                .set_level(request.params)
                .await
                .map(ServerResult::empty),
            ClientRequest::GetPromptRequest(request) => self
                .handler
                .get_prompt(request.params)
                .await
                .map(ServerResult::GetPromptResult),
            ClientRequest::ListPromptsRequest(request) => self
                .handler
                .list_prompts(request.params)
                .await
                .map(ServerResult::ListPromptsResult),
            ClientRequest::ListResourcesRequest(request) => self
                .handler
                .list_resources(request.params)
                .await
                .map(ServerResult::ListResourcesResult),
            ClientRequest::ListResourceTemplatesRequest(request) => self
                .handler
                .list_resource_templates(request.params)
                .await
                .map(ServerResult::ListResourceTemplatesResult),
            ClientRequest::ReadResourceRequest(request) => self
                .handler
                .read_resource(request.params)
                .await
                .map(ServerResult::ReadResourceResult),
            ClientRequest::SubscribeRequest(request) => self
                .handler
                .subscribe(request.params)
                .await
                .map(ServerResult::empty),
            ClientRequest::UnsubscribeRequest(request) => self
                .handler
                .unsubscribe(request.params)
                .await
                .map(ServerResult::empty),
            ClientRequest::CallToolRequest(request) => self
                .handler
                .call_tool(request.params)
                .await
                .map(ServerResult::CallToolResult),
            ClientRequest::ListToolsRequest(request) => self
                .handler
                .list_tools(request.params)
                .await
                .map(ServerResult::ListToolsResult),
        };
        result.map_err(ServiceError::McpError)
    }

    async fn handle_notification(
        &self,
        notification: <Self::Role as mcp_core::service::ServiceRole>::PeerNot,
    ) -> Result<(), ServiceError> {
        match notification {
            ClientNotification::CancelledNotification(notification) => {
                self.handler.on_cancelled(notification.params).await
            }
            ClientNotification::ProgressNotification(notification) => {
                self.handler.on_progress(notification.params).await
            }
            ClientNotification::InitializedNotification(_notification) => {
                self.handler.on_initialized().await
            }
            ClientNotification::RootsListChangedNotification(_notification) => {
                self.handler.on_roots_list_changed().await
            }
        };
        Ok(())
    }

    fn get_peer_proxy(&self) -> Option<PeerProxy<Self::Role>> {
        self.handler.get_peer_proxy()
    }

    fn set_peer_proxy(&mut self, peer: PeerProxy<Self::Role>) {
        self.handler.set_peer_proxy(peer);
    }
}

#[allow(unused_variables)]
pub trait Handler: Sized + Send {
    // handle requests
    fn initialize(
        &self,
        request: InitializeRequestParam,
    ) -> impl Future<Output = Result<InitializeResult, McpError>> + Send + '_ {
        std::future::ready(Ok(InitializeResult {
            protocol_version: LatestProtocolVersion,
            capabilities: ServerCapabilities::default(),
            server_info: Implementation {
                name: env!("CARGO_CRATE_NAME").to_owned(),
                version: env!("CARGO_PKG_VERSION").to_owned(),
            },
            instructions: None,
        }))
    }
    fn complete(
        &self,
        request: CompleteRequestParam,
    ) -> impl Future<Output = Result<CompleteResult, McpError>> + Send + '_ {
        std::future::ready(Err(McpError::not_found()))
    }
    fn set_level(
        &self,
        request: SetLevelRequestParam,
    ) -> impl Future<Output = Result<(), McpError>> + Send + '_ {
        std::future::ready(Err(McpError::not_found()))
    }
    fn get_prompt(
        &self,
        request: GetPromptRequestParam,
    ) -> impl Future<Output = Result<GetPromptResult, McpError>> + Send + '_ {
        std::future::ready(Err(McpError::not_found()))
    }
    fn list_prompts(
        &self,
        request: PaginatedRequestParam,
    ) -> impl Future<Output = Result<ListPromptsResult, McpError>> + Send + '_ {
        std::future::ready(Err(McpError::not_found()))
    }
    fn list_resources(
        &self,
        request: PaginatedRequestParam,
    ) -> impl Future<Output = Result<ListResourcesResult, McpError>> + Send + '_ {
        std::future::ready(Err(McpError::not_found()))
    }
    fn list_resource_templates(
        &self,
        request: PaginatedRequestParam,
    ) -> impl Future<Output = Result<ListResourceTemplatesResult, McpError>> + Send + '_ {
        std::future::ready(Err(McpError::not_found()))
    }
    fn read_resource(
        &self,
        request: ReadResourceRequestParam,
    ) -> impl Future<Output = Result<ReadResourceResult, McpError>> + Send + '_ {
        std::future::ready(Err(McpError::not_found()))
    }
    fn subscribe(
        &self,
        request: SubscribeRequestParam,
    ) -> impl Future<Output = Result<(), McpError>> + Send + '_ {
        std::future::ready(Err(McpError::not_found()))
    }
    fn unsubscribe(
        &self,
        request: UnsubscribeRequestParam,
    ) -> impl Future<Output = Result<(), McpError>> + Send + '_ {
        std::future::ready(Err(McpError::not_found()))
    }
    fn call_tool(
        &self,
        request: CallToolRequestParam,
    ) -> impl Future<Output = Result<CallToolResult, McpError>> + Send + '_ {
        std::future::ready(Err(McpError::not_found()))
    }
    fn list_tools(
        &self,
        request: PaginatedRequestParam,
    ) -> impl Future<Output = Result<ListToolsResult, McpError>> + Send + '_ {
        std::future::ready(Err(McpError::not_found()))
    }

    // handle notifications

    fn on_cancelled(
        &self,
        notification: CancelledNotificationParam,
    ) -> impl Future<Output = ()> + Send + '_ {
        std::future::ready(())
    }
    fn on_progress(
        &self,
        notification: ProgressNotificationParam,
    ) -> impl Future<Output = ()> + Send + '_ {
        std::future::ready(())
    }
    fn on_initialized(&self) -> impl Future<Output = ()> + Send + '_ {
        std::future::ready(())
    }
    fn on_roots_list_changed(&self) -> impl Future<Output = ()> + Send + '_ {
        std::future::ready(())
    }

    fn get_peer_proxy(&self) -> Option<PeerProxy<RoleServer>> {
        None
    }

    fn set_peer_proxy(&mut self, peer: PeerProxy<RoleServer>) {
        drop(peer);
    }
}
