use mcp_core::error::Error as McpError;
use mcp_core::schema::*;
use mcp_core::service::{Peer, RoleClient, Service, ServiceError};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct ClientHandlerService<H> {
    pub handler: H,
}

impl<H: ClientHandler> ClientHandlerService<H> {
    pub fn new(handler: H) -> Self {
        Self { handler }
    }
}

impl<H: ClientHandler> Service for ClientHandlerService<H> {
    type Role = RoleClient;

    async fn handle_request(
        &self,
        request: <Self::Role as mcp_core::service::ServiceRole>::PeerReq,
    ) -> Result<<Self::Role as mcp_core::service::ServiceRole>::Resp, McpError> {
        match request {
            ServerRequest::PingRequest(_) => self.handler.ping().await.map(ClientResult::empty),
            ServerRequest::CreateMessageRequest(request) => self
                .handler
                .create_message(request.params)
                .await
                .map(ClientResult::CreateMessageResult),
            ServerRequest::ListRootsRequest(_) => self
                .handler
                .list_roots()
                .await
                .map(ClientResult::ListRootsResult),
        }
    }

    async fn handle_notification(
        &self,
        notification: <Self::Role as mcp_core::service::ServiceRole>::PeerNot,
    ) -> Result<(), McpError> {
        match notification {
            ServerNotification::CancelledNotification(notification) => {
                self.handler.on_cancelled(notification.params).await
            }
            ServerNotification::ProgressNotification(notification) => {
                self.handler.on_progress(notification.params).await
            }
            ServerNotification::LoggingMessageNotification(notification) => {
                self.handler.on_logging_message(notification.params).await
            }
            ServerNotification::ResourceUpdatedNotification(notification) => {
                self.handler.on_resource_updated(notification.params).await
            }
            ServerNotification::ResourceListChangedNotification(_notification_no_param) => {
                self.handler.on_resource_list_changed().await
            }
            ServerNotification::ToolListChangedNotification(_notification_no_param) => {
                self.handler.on_tool_list_changed().await
            }
            ServerNotification::PromptListChangedNotification(_notification_no_param) => {
                self.handler.on_prompt_list_changed().await
            }
        };
        Ok(())
    }

    fn get_peer(&self) -> Option<Peer<Self::Role>> {
        self.handler.get_peer()
    }

    fn set_peer(&mut self, peer: Peer<Self::Role>) {
        self.handler.set_peer(peer);
    }
    
    fn set_peer_info(&mut self, peer: <Self::Role as mcp_core::service::ServiceRole>::PeerInfo) {
        self.handler.set_peer_info(peer);
    }
    
    fn get_peer_info(&self) -> Option<<Self::Role as mcp_core::service::ServiceRole>::PeerInfo> {
        self.handler.get_peer_info()
    }
    
    fn get_info(&self) -> <Self::Role as mcp_core::service::ServiceRole>::Info {
        self.handler.get_info()
    }
}

#[allow(unused_variables)]
pub trait ClientHandler: Sized + Send + Sync + 'static {
    fn ping(&self) -> impl Future<Output = Result<(), McpError>> + Send + '_ {
        std::future::ready(Ok(()))
    }

    fn create_message(
        &self,
        params: CreateMessageRequestParam,
    ) -> impl Future<Output = Result<CreateMessageResult, McpError>> + Send + '_ {
        std::future::ready(Err(
            McpError::method_not_found::<CreateMessageRequestMethod>(),
        ))
    }
    fn list_roots(&self) -> impl Future<Output = Result<ListRootsResult, McpError>> + Send + '_ {
        std::future::ready(Err(McpError::method_not_found::<ListRootsRequestMethod>()))
    }

    fn on_cancelled(
        &self,
        params: CancelledNotificationParam,
    ) -> impl Future<Output = ()> + Send + '_ {
        std::future::ready(())
    }
    fn on_progress(
        &self,
        params: ProgressNotificationParam,
    ) -> impl Future<Output = ()> + Send + '_ {
        std::future::ready(())
    }
    fn on_logging_message(
        &self,
        params: LoggingMessageNotificationParam,
    ) -> impl Future<Output = ()> + Send + '_ {
        std::future::ready(())
    }
    fn on_resource_updated(
        &self,
        params: ResourceUpdatedNotificationParam,
    ) -> impl Future<Output = ()> + Send + '_ {
        std::future::ready(())
    }
    fn on_resource_list_changed(&self) -> impl Future<Output = ()> + Send + '_ {
        std::future::ready(())
    }
    fn on_tool_list_changed(&self) -> impl Future<Output = ()> + Send + '_ {
        std::future::ready(())
    }
    fn on_prompt_list_changed(&self) -> impl Future<Output = ()> + Send + '_ {
        std::future::ready(())
    }

    // method!(proxy_req initialize InitializeRequest(InitializeRequestParam) => InitializeResult);
    // method!(proxy_req request_complete CompleteRequest(CompleteRequestParam) => CompleteResult);
    // method!(proxy_req request_set_level SetLevelRequest(SetLevelRequestParam));
    // method!(proxy_req request_get_prompt GetPromptRequest(GetPromptRequestParam) => GetPromptResult);
    // method!(proxy_req request_list_prompts ListPromptsRequest(PaginatedRequestParam) => ListPromptsResult);
    // method!(proxy_req request_list_resources ListResourcesRequest(PaginatedRequestParam) => ListResourcesResult);
    // method!(proxy_req request_list_resource_templates ListResourceTemplatesRequest(PaginatedRequestParam) => ListResourceTemplatesResult);
    // method!(proxy_req request_read_resource ReadResourceRequest(ReadResourceRequestParam) => ReadResourceResult);
    // method!(proxy_req request_subscribe SubscribeRequest(SubscribeRequestParam) );
    // method!(proxy_req request_unsubscribe UnsubscribeRequest(UnsubscribeRequestParam));
    // method!(proxy_req request_call_tool CallToolRequest(CallToolRequestParam) => CallToolResult);
    // method!(proxy_req request_list_tools ListToolsRequest(PaginatedRequestParam) => ListToolsResult);

    // method!(proxy_not notify_cancelled CancelledNotification(CancelledNotificationParam));
    // method!(proxy_not notify_progress ProgressNotification(ProgressNotificationParam));
    // method!(proxy_not notify_initialized InitializedNotification);
    // method!(proxy_not notify_roots_list_changed RootsListChangedNotification);


    fn get_peer(&self) -> Option<Peer<RoleClient>>;

    fn set_peer(&mut self, peer: Peer<RoleClient>);

    fn set_peer_info(&mut self, peer: ServerInfo) {
        drop(peer);
    }
    
    fn get_peer_info(&self) -> Option<ServerInfo> {
        None
    }
    
    fn get_info(&self) -> ClientInfo {
        ClientInfo::default()
    }
}
