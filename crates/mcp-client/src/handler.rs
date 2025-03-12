use mcp_core::error::Error as McpError;
use mcp_core::schema::*;
use mcp_core::service::{PeerProxy, RoleClient, Service, ServiceError};

macro_rules! method {
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
                    .send_request(ClientRequest::$Req($Req {
                        method: Default::default(),
                        params,
                    }))
                    .await?;
                match result {
                    ServerResult::$Resp(result) => Ok(result),
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
                    .send_request(ClientRequest::$Req($Req {
                        method: Default::default(),
                        params,
                    }))
                    .await?;
                match result {
                    ServerResult::EmptyResult(_) => Ok(()),
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
                    .send_notification(ClientNotification::$Not(
                        $Not {
                            method: Default::default(),
                            params,
                        },
                    ))
                    .await?;
                Ok(())
            }
        }
    };
    (proxy_not $method:ident $Not:ident) => {
        fn $method(
            &self,
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
                    .send_notification(ClientNotification::$Not(
                        $Not {
                            method: Default::default(),
                        },
                    ))
                    .await?;
                Ok(())
            }
        }
    };
}

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
        let result = match request {
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
        };
        result
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

    fn get_peer_proxy(&self) -> Option<PeerProxy<Self::Role>> {
        self.handler.get_peer_proxy()
    }

    fn set_peer_proxy(&mut self, peer: PeerProxy<Self::Role>) {
        self.handler.set_peer_proxy(peer);
    }
}

#[allow(unused_variables)]
pub trait ClientHandler: Sized + Send {
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

    fn initialize(
        &self,
        params: InitializeRequestParam,
    ) -> impl Future<Output = Result<InitializeResult, McpError>> + Send + '_ {
        std::future::ready(Ok(InitializeResult {
            protocol_version: LatestProtocolVersion,
            capabilities: ServerCapabilities::default(),
            server_info: Implementation::from_build_env(),
            instructions: None,
        }))
    }


    method!(proxy_req request_complete CompleteRequest(CompleteRequestParam) => CompleteResult);
    method!(proxy_req request_set_level SetLevelRequest(SetLevelRequestParam));
    method!(proxy_req request_get_prompt GetPromptRequest(GetPromptRequestParam) => GetPromptResult);
    method!(proxy_req request_list_prompts ListPromptsRequest(PaginatedRequestParam) => ListPromptsResult);
    method!(proxy_req request_list_resources ListResourcesRequest(PaginatedRequestParam) => ListResourcesResult);
    method!(proxy_req request_list_resource_templates ListResourceTemplatesRequest(PaginatedRequestParam) => ListResourceTemplatesResult);
    method!(proxy_req request_read_resource ReadResourceRequest(ReadResourceRequestParam) => ReadResourceResult);
    method!(proxy_req request_subscribe SubscribeRequest(SubscribeRequestParam) );
    method!(proxy_req request_unsubscribe UnsubscribeRequest(UnsubscribeRequestParam));
    method!(proxy_req request_call_tool CallToolRequest(CallToolRequestParam) => CallToolResult);
    method!(proxy_req request_list_tools ListToolsRequest(PaginatedRequestParam) => ListToolsResult);

    method!(proxy_not notify_cancelled CancelledNotification(CancelledNotificationParam));
    method!(proxy_not notify_progress ProgressNotification(ProgressNotificationParam));
    method!(proxy_not notify_initialized InitializedNotification);
    method!(proxy_not notify_roots_list_changed RootsListChangedNotification);


    fn get_peer_proxy(&self) -> Option<PeerProxy<RoleClient>> {
        None
    }

    fn set_peer_proxy(&mut self, peer: PeerProxy<RoleClient>) {
        drop(peer);
    }
}
