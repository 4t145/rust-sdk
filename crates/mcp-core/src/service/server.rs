use super::*;
use crate::error::Error as McpError;
use crate::schema::*;
use futures::{Sink, SinkExt, Stream, StreamExt};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RoleServer;

impl ServiceRole for RoleServer {
    type Req = ServerRequest;
    type Resp = ServerResult;
    type Not = ServerNotification;
    type PeerReq = ClientRequest;
    type PeerResp = ClientResult;
    type PeerNot = ClientNotification;
    type Info = ServerInfo;
    type PeerInfo = ClientInfo;
    const IS_CLIENT: bool = false;
}

pub type ClientSink = Peer<RoleServer>;

pub async fn serve_server<S, T, E>(service: S, transport: T) -> Result<RunningService<S, E>, E>
where
    S: Service<Role = RoleServer>,
    T: Stream<Item = ClientJsonRpcMessage> + Sink<ServerJsonRpcMessage, Error = E> + Send + Unpin + 'static,
    E: From<std::io::Error> + Send + 'static,
{
    // service
    serve_inner(service, transport, async |s, t, _| {
        let (request, id) = t
            .next()
            .await
            .ok_or(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "expect initialize request",
            ))?
            .into_message()
            .into_request()
            .ok_or(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "expect initialize request",
            ))?;
        let ClientRequest::InitializeRequest(peer_info) = request else {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "expect initialize request",
            )
            .into());
        };
        let init_response = s.get_info();
        t.send(
            ServerMessage::Response(ServerResult::InitializeResult(init_response), id)
                .into_json_rpc_message(),
        )
        .await?;
        // waiting for notification
        let notification = t
            .next()
            .await
            .ok_or(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "expect initialize notification",
            ))?
            .into_message()
            .into_notification()
            .ok_or(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "expect initialize notification",
            ))?;
        let ClientNotification::InitializedNotification(_) = notification else {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "expect initialize notification",
            )
            .into());
        };
        s.set_peer_info(peer_info.params.clone());
        Ok(peer_info.params)
    })
    .await
}

macro_rules! method {
    (proxy_req $method:ident $Req:ident() => $Resp: ident ) => {
        pub async fn $method(&self) -> Result<$Resp, ServiceError> {
            let result = self
                .send_request(ServerRequest::$Req($Req {
                    method: Default::default(),
                }))
                .await?;
            match result {
                ClientResult::$Resp(result) => Ok(result),
                _ => Err(ServiceError::UnexpectedResponse),
            }
        }
    };
    (proxy_req $method:ident $Req:ident($Param: ident) => $Resp: ident ) => {
        pub async fn $method(&self, params: $Param) -> Result<$Resp, ServiceError> {
            let result = self
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
    };
    (proxy_req $method:ident $Req:ident($Param: ident)) => {
        pub fn $method(
            &self,
            params: $Param,
        ) -> impl Future<Output = Result<(), ServiceError>> + Send + '_ {
            async move {
                let result = self
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
        pub async fn $method(&self, params: $Param) -> Result<(), ServiceError> {
            self.send_notification(ServerNotification::$Not($Not {
                method: Default::default(),
                params,
            }))
            .await?;
            Ok(())
        }
    };
    (proxy_not $method:ident $Not:ident) => {
        pub async fn $method(&self) -> Result<(), ServiceError> {
            self.send_notification(ServerNotification::$Not($Not {
                method: Default::default(),
            }))
            .await?;
            Ok(())
        }
    };
}

impl Peer<RoleServer> {
    method!(proxy_req create_message CreateMessageRequest(CreateMessageRequestParam) => CreateMessageResult);
    method!(proxy_req list_roots ListRootsRequest() => ListRootsResult);

    method!(proxy_not notify_cancelled CancelledNotification(CancelledNotificationParam));
    method!(proxy_not notify_progress ProgressNotification(ProgressNotificationParam));
    method!(proxy_not notify_logging_message LoggingMessageNotification(LoggingMessageNotificationParam));
    method!(proxy_not notify_resource_updated ResourceUpdatedNotification(ResourceUpdatedNotificationParam));
    method!(proxy_not notify_resource_list_changed ResourceListChangedNotification);
    method!(proxy_not notify_tool_list_changed ToolListChangedNotification);
    method!(proxy_not notify_prompt_list_changed PromptListChangedNotification);
}
