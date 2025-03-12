use mcp_core::error::Error as McpError;
use mcp_core::schema::*;
use mcp_core::service::{PeerProxy, RoleServer, Service, ServiceError};

pub mod tool;

macro_rules! method {
    (proxy_req $method:ident $Req:ident() => $Resp: ident ) => {
        fn $method(&self) -> impl Future<Output = Result<$Resp, ServiceError>> + Send + '_
        where
            Self: Sync,
        {
            async move {
                let Some(proxy) = self.get_peer_proxy() else {
                    return Err(ServiceError::Transport(std::io::Error::other(
                        "peer proxy not initialized",
                    )));
                };
                let result = proxy
                    .send_request(ServerRequest::$Req($Req {
                        method: Default::default(),
                    }))
                    .await?;
                match result {
                    ClientResult::$Resp(result) => Ok(result),
                    _ => Err(ServiceError::UnexpectedResponse),
                }
            }
        }
    };
    (proxy_req $method:ident $Req:ident($Param: ident) => $Resp: ident ) => {
        fn $method(
            &self,
            params: $Param,
        ) -> impl Future<Output = Result<$Resp, ServiceError>> + Send + '_
        where
            Self: Sync,
        {
            async move {
                let Some(proxy) = self.get_peer_proxy() else {
                    return Err(ServiceError::Transport(std::io::Error::other(
                        "peer proxy not initialized",
                    )));
                };
                let result = proxy
                    .send_request(ServerRequest::$Req($Req {
                        method: Default::default(),
                        params,
                    }))
                    .await?;
                match result {
                    ClientResult::$Resp(result) => Ok(result),
                    _ => Err(ServiceError::UnexpectedResponse),
                }
            }
        }
    };
    (proxy_req $method:ident $Req:ident($Param: ident)) => {
        fn $method(
            &self,
            params: $Param,
        ) -> impl Future<Output = Result<(), ServiceError>> + Send + '_
        where
            Self: Sync,
        {
            async move {
                let Some(proxy) = self.get_peer_proxy() else {
                    return Err(ServiceError::Transport(std::io::Error::other(
                        "peer proxy not initialized",
                    )));
                };
                let result = proxy
                    .send_request(ServerRequest::$Req($Req {
                        method: Default::default(),
                        params,
                    }))
                    .await?;
                match result {
                    ClientResult::EmptyResult(_) => Ok(()),
                    _ => Err(ServiceError::UnexpectedResponse),
                }
            }
        }
    };

    (proxy_not $method:ident $Not:ident($Param: ident)) => {
        fn $method(
            &self,
            params: $Param,
        ) -> impl Future<Output = Result<(), ServiceError>> + Send + '_
        where
            Self: Sync,
        {
            async move {
                let Some(proxy) = self.get_peer_proxy() else {
                    return Err(ServiceError::Transport(std::io::Error::other(
                        "peer proxy not initialized",
                    )));
                };
                proxy
                    .send_notification(ServerNotification::$Not($Not {
                        method: Default::default(),
                        params,
                    }))
                    .await?;
                Ok(())
            }
        }
    };
    (proxy_not $method:ident $Not:ident) => {
        fn $method(&self) -> impl Future<Output = Result<(), ServiceError>> + Send + '_
        where
            Self: Sync,
        {
            async move {
                let Some(proxy) = self.get_peer_proxy() else {
                    return Err(ServiceError::Transport(std::io::Error::other(
                        "peer proxy not initialized",
                    )));
                };
                proxy
                    .send_notification(ServerNotification::$Not($Not {
                        method: Default::default(),
                    }))
                    .await?;
                Ok(())
            }
        }
    };
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct ServerHandlerService<H> {
    pub handler: H,
}

impl<H: ServerHandler> ServerHandlerService<H> {
    pub fn new(handler: H) -> Self {
        Self { handler }
    }
}

impl<H: ServerHandler> Service for ServerHandlerService<H> {
    type Role = RoleServer;

    async fn handle_request(
        &self,
        request: <Self::Role as mcp_core::service::ServiceRole>::PeerReq,
    ) -> Result<<Self::Role as mcp_core::service::ServiceRole>::Resp, McpError> {
        match request {
            ClientRequest::PingRequest(_request) => {
                self.handler.ping().await.map(ServerResult::empty)
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
        }
    }

    async fn handle_notification(
        &self,
        notification: <Self::Role as mcp_core::service::ServiceRole>::PeerNot,
    ) -> Result<(), McpError> {
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
pub trait ServerHandler: Sized + Send {
    fn ping(&self) -> impl Future<Output = Result<(), McpError>> + Send + '_ {
        std::future::ready(Ok(()))
    }
    // handle requests
    fn initialize(
        &self,
        request: InitializeRequestParam,
    ) -> impl Future<Output = Result<InitializeResult, McpError>> + Send + '_ {
        std::future::ready(Ok(InitializeResult {
            protocol_version: LatestProtocolVersion,
            capabilities: ServerCapabilities::default(),
            server_info: Implementation::from_build_env(),
            instructions: None,
        }))
    }
    fn complete(
        &self,
        request: CompleteRequestParam,
    ) -> impl Future<Output = Result<CompleteResult, McpError>> + Send + '_ {
        std::future::ready(Err(McpError::method_not_found::<CompleteRequestMethod>()))
    }
    fn set_level(
        &self,
        request: SetLevelRequestParam,
    ) -> impl Future<Output = Result<(), McpError>> + Send + '_ {
        std::future::ready(Err(McpError::method_not_found::<SetLevelRequestMethod>()))
    }
    fn get_prompt(
        &self,
        request: GetPromptRequestParam,
    ) -> impl Future<Output = Result<GetPromptResult, McpError>> + Send + '_ {
        std::future::ready(Err(McpError::method_not_found::<GetPromptRequestMethod>()))
    }
    fn list_prompts(
        &self,
        request: PaginatedRequestParam,
    ) -> impl Future<Output = Result<ListPromptsResult, McpError>> + Send + '_ {
        std::future::ready(Err(McpError::method_not_found::<ListPromptsRequestMethod>()))
    }
    fn list_resources(
        &self,
        request: PaginatedRequestParam,
    ) -> impl Future<Output = Result<ListResourcesResult, McpError>> + Send + '_ {
        std::future::ready(Err(
            McpError::method_not_found::<ListResourcesRequestMethod>(),
        ))
    }
    fn list_resource_templates(
        &self,
        request: PaginatedRequestParam,
    ) -> impl Future<Output = Result<ListResourceTemplatesResult, McpError>> + Send + '_ {
        std::future::ready(Err(McpError::method_not_found::<
            ListResourceTemplatesRequestMethod,
        >()))
    }
    fn read_resource(
        &self,
        request: ReadResourceRequestParam,
    ) -> impl Future<Output = Result<ReadResourceResult, McpError>> + Send + '_ {
        std::future::ready(Err(
            McpError::method_not_found::<ReadResourceRequestMethod>(),
        ))
    }
    fn subscribe(
        &self,
        request: SubscribeRequestParam,
    ) -> impl Future<Output = Result<(), McpError>> + Send + '_ {
        std::future::ready(Err(McpError::method_not_found::<SubscribeRequestMethod>()))
    }
    fn unsubscribe(
        &self,
        request: UnsubscribeRequestParam,
    ) -> impl Future<Output = Result<(), McpError>> + Send + '_ {
        std::future::ready(Err(McpError::method_not_found::<UnsubscribeRequestMethod>()))
    }
    fn call_tool(
        &self,
        request: CallToolRequestParam,
    ) -> impl Future<Output = Result<CallToolResult, McpError>> + Send + '_ {
        std::future::ready(Err(McpError::method_not_found::<CallToolRequestMethod>()))
    }
    fn list_tools(
        &self,
        request: PaginatedRequestParam,
    ) -> impl Future<Output = Result<ListToolsResult, McpError>> + Send + '_ {
        std::future::ready(Err(McpError::method_not_found::<ListToolsRequestMethod>()))
    }

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
        tracing::info!("client initialized");
        std::future::ready(())
    }
    fn on_roots_list_changed(&self) -> impl Future<Output = ()> + Send + '_ {
        std::future::ready(())
    }

    method!(proxy_req create_message CreateMessageRequest(CreateMessageRequestParam) => CreateMessageResult);
    method!(proxy_req list_roots ListRootsRequest() => ListRootsResult);

    method!(proxy_not notify_cancelled CancelledNotification(CancelledNotificationParam));
    method!(proxy_not notify_progress ProgressNotification(ProgressNotificationParam));
    method!(proxy_not notify_logging_message LoggingMessageNotification(LoggingMessageNotificationParam));
    method!(proxy_not notify_resource_updated ResourceUpdatedNotification(ResourceUpdatedNotificationParam));
    method!(proxy_not notify_resource_list_changed ResourceListChangedNotification);
    method!(proxy_not notify_tool_list_changed ToolListChangedNotification);
    method!(proxy_not notify_prompt_list_changed PromptListChangedNotification);

    fn get_peer_proxy(&self) -> Option<PeerProxy<RoleServer>> {
        None
    }

    fn set_peer_proxy(&mut self, peer: PeerProxy<RoleServer>) {
        drop(peer);
    }
}
