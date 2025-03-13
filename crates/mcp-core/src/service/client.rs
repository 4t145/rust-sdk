use crate::schema::*;
use futures::{Sink, SinkExt, Stream, StreamExt};
use super::*;


#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RoleClient;


impl ServiceRole for RoleClient {
    type Req = ClientRequest;
    type Resp = ClientResult;
    type Not = ClientNotification;
    type PeerReq = ServerRequest;
    type PeerResp = ServerResult;
    type PeerNot = ServerNotification;
    type Info = ClientInfo;
    type PeerInfo = ServerInfo;

    const IS_CLIENT: bool = true;
}

pub type ServerSink = Peer<RoleClient>;


pub async fn serve_client<S, T, E>(service: S, transport: T) -> Result<RunningService<S, E>, E>
where
    S: Service<Role = RoleClient>,
    T: Stream<Item = ServerJsonRpcMessage> + Sink<ClientJsonRpcMessage, Error = E> + Send + Unpin + 'static,
    E: From<std::io::Error> + Send + 'static,
{
    // service
    serve_inner(service, transport, async |s, t, id_provider| {
        let id = id_provider.next_request_id();
        let init_request = InitializeRequest {
            method: Default::default(),
            params: s.get_info(),
        };
        t.send(
            ClientMessage::Request(ClientRequest::InitializeRequest(init_request), id.clone())
                .into_json_rpc_message(),
        )
        .await?;
        let (response, response_id) = t
            .next()
            .await
            .ok_or(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "expect initialize response",
            ))?
            .into_message()
            .into_result()
            .ok_or(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "expect initialize result",
            ))?;
        if id != response_id {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "conflict initialize response id",
            )
            .into());
        }
        let response = response.map_err(std::io::Error::other)?;
        let ServerResult::InitializeResult(initialize_result) = response else {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "expect initialize result",
            )
            .into());
        };
        // send notification
        let notification = ClientMessage::Notification(
            ClientNotification::InitializedNotification(InitializedNotification {
                method: Default::default(),
            }),
        );
        t.send(notification.into_json_rpc_message()).await?;
        s.set_peer_info(initialize_result.clone());
        Ok(initialize_result)
    })
    .await
}


macro_rules! method {
    (proxy_req $method:ident $Req:ident() => $Resp: ident ) => {
        pub async fn $method(&self) -> Result<$Resp, ServiceError> {
            let result = self
                .send_request(ClientRequest::$Req($Req {
                    method: Default::default(),
                }))
                .await?;
            match result {
                ServerResult::$Resp(result) => Ok(result),
                _ => Err(ServiceError::UnexpectedResponse),
            }
        }
    };
    (proxy_req $method:ident $Req:ident($Param: ident) => $Resp: ident ) => {
        pub async fn $method(&self, params: $Param) -> Result<$Resp, ServiceError> {
            let result = self
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
    };
    (proxy_req $method:ident $Req:ident($Param: ident)) => {
        pub async fn $method(
            &self,
            params: $Param,
        ) -> Result<(), ServiceError> {
        let result = self
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
    };

    (proxy_not $method:ident $Not:ident($Param: ident)) => {
        pub async fn $method(&self, params: $Param) -> Result<(), ServiceError> {
            self.send_notification(ClientNotification::$Not($Not {
                method: Default::default(),
                params,
            }))
            .await?;
            Ok(())
        }
    };
    (proxy_not $method:ident $Not:ident) => {
        pub async fn $method(&self) -> Result<(), ServiceError> {
            self.send_notification(ClientNotification::$Not($Not {
                method: Default::default(),
            }))
            .await?;
            Ok(())
        }
    };
}


impl Peer<RoleClient> {
   method!(proxy_req complete CompleteRequest(CompleteRequestParam) => CompleteResult);
    method!(proxy_req set_level SetLevelRequest(SetLevelRequestParam));
    method!(proxy_req get_prompt GetPromptRequest(GetPromptRequestParam) => GetPromptResult);
    method!(proxy_req list_prompts ListPromptsRequest(PaginatedRequestParam) => ListPromptsResult);
    method!(proxy_req list_resources ListResourcesRequest(PaginatedRequestParam) => ListResourcesResult);
    method!(proxy_req list_resource_templates ListResourceTemplatesRequest(PaginatedRequestParam) => ListResourceTemplatesResult);
    method!(proxy_req read_resource ReadResourceRequest(ReadResourceRequestParam) => ReadResourceResult);
    method!(proxy_req subscribe SubscribeRequest(SubscribeRequestParam) );
    method!(proxy_req unsubscribe UnsubscribeRequest(UnsubscribeRequestParam));
    method!(proxy_req call_tool CallToolRequest(CallToolRequestParam) => CallToolResult);
    method!(proxy_req list_tools ListToolsRequest(PaginatedRequestParam) => ListToolsResult);

    method!(proxy_not notify_cancelled CancelledNotification(CancelledNotificationParam));
    method!(proxy_not notify_progress ProgressNotification(ProgressNotificationParam));
    method!(proxy_not notify_initialized InitializedNotification);
    method!(proxy_not notify_roots_list_changed RootsListChangedNotification);
}