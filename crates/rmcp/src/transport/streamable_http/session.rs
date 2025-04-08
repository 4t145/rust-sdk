use std::{
    borrow::Cow,
    collections::{HashMap, VecDeque},
    num::ParseIntError,
    sync::Arc,
};

use serde::de;
use thiserror::Error;
use tokio::sync::{
    mpsc::{Receiver, Sender},
    oneshot,
};

use crate::model::{
    CancelledNotificationParam, ClientJsonRpcMessage, ClientRequest, GetMeta, InitializeRequest,
    InitializedNotification, JsonRpcNotification, JsonRpcRequest, Notification,
    ProgressNotificationParam, ProgressToken, RequestId, ServerJsonRpcMessage, ServerNotification,
};

pub const HEADER_SESSION_ID: &str = "Mcp-Session-Id";
#[derive(Debug, Clone)]
pub struct ServerSessionMessage {
    pub event_id: EventId,
    pub message: Arc<ServerJsonRpcMessage>,
}

/// <index>-<[n|s]>-<request_id>
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct EventId {
    request_id: Option<RequestId>,
    index: usize,
}

impl std::fmt::Display for EventId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.index)?;
        match &self.request_id {
            Some(crate::model::NumberOrString::Number(number)) => write!(f, "-n-{}", number),
            Some(crate::model::NumberOrString::String(string)) => write!(f, "-s-{}", string),
            None => write!(f, "-c"),
        }
    }
}

#[derive(Debug, Clone, Error)]
pub enum EventIdParseError {
    #[error("Invalid index: {0}")]
    InvalidIndex(ParseIntError),
    #[error("Invalid numeric request id: {0}")]
    InvalidNumericRequestId(ParseIntError),
    #[error("Missing request id type")]
    InvalidRequestIdType,
    #[error("Missing request id")]
    MissingRequestId,
}

impl std::str::FromStr for EventId {
    type Err = EventIdParseError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (index, maybe_request_id) = s
            .split_once("-")
            .ok_or(EventIdParseError::MissingRequestId)?;
        let index = usize::from_str(index).map_err(EventIdParseError::InvalidIndex)?;
        let request_id = if let Some(number) = maybe_request_id.strip_prefix("n-") {
            let number = number
                .parse::<u32>()
                .map_err(EventIdParseError::InvalidNumericRequestId)?;
            Some(crate::model::NumberOrString::Number(number))
        } else if let Some(string) = maybe_request_id.strip_prefix("s-") {
            let string = string.to_string();
            Some(crate::model::NumberOrString::String(string.into()))
        } else if "c" == maybe_request_id {
            None
        } else {
            return Err(EventIdParseError::InvalidRequestIdType);
        };
        Ok(EventId { request_id, index })
    }
}

type SessionId = String;

struct CachedTx {
    tx: Sender<ServerSessionMessage>,
    cache: VecDeque<ServerSessionMessage>,
    request_id: Option<RequestId>,
    capacity: usize,
}

impl CachedTx {
    fn new(tx: Sender<ServerSessionMessage>, request_id: Option<RequestId>) -> Self {
        Self {
            cache: VecDeque::with_capacity(tx.capacity()),
            capacity: tx.capacity(),
            tx,
            request_id,
        }
    }
    fn new_common(tx: Sender<ServerSessionMessage>) -> Self {
        Self::new(tx, None)
    }

    async fn send(&mut self, message: ServerJsonRpcMessage) {
        let index = self.cache.back().map_or(0, |m| m.event_id.index + 1);
        let event_id = EventId {
            request_id: self.request_id.clone(),
            index,
        };
        let message = ServerSessionMessage {
            event_id: event_id.clone(),
            message: Arc::new(message),
        };
        if self.cache.len() >= self.capacity {
            self.cache.pop_front();
            self.cache.push_back(message.clone());
        } else {
            self.cache.push_back(message.clone());
        }
        let _ = self.tx.send(message).await;
    }

    async fn sync(&mut self, index: usize) -> Result<(), SessionError> {
        let Some(front) = self.cache.front() else {
            return Ok(());
        };
        let sync_index = index.wrapping_sub(front.event_id.index);
        if sync_index > self.cache.len() {
            // invalid index
            return Err(SessionError::InvalidEventId);
        }
        for message in self.cache.iter().skip(sync_index) {
            let send_result = self.tx.send(message.clone()).await;
            if send_result.is_err() {
                return Err(SessionError::ChannelClosed(
                    message.event_id.request_id.clone(),
                ));
            }
        }
        Ok(())
    }
}

struct RequestWise {
    progress_token: Option<ProgressToken>,
    tx: CachedTx,
}

pub struct SessionContext {
    id: SessionId,
    request_router: HashMap<RequestId, RequestWise>,
    common: CachedTx,
    // ProgressToken - RequestId map
    // pt_rid_map: HashMap<ProgressToken, RequestId>,
    to_service_tx: Sender<ClientJsonRpcMessage>,
    event_rx: Receiver<SessionEvent>,
}

pub enum SessionError {
    DuplicatedRequestId(RequestId),
    ChannelClosed(Option<RequestId>),
    EventIdParseError(Cow<'static, str>),
    SessionServiceTerminated,
    InvalidEventId,
    TransportClosed,
}

enum OutboundChannel {
    RequestWise { id: RequestId, close: bool },
    Common,
}

impl SessionContext {
    const REQUEST_WISE_CHANNEL_SIZE: usize = 16;
    pub fn session_id(&self) -> &SessionId {
        &self.id
    }
    pub async fn send_to_service(&self, message: ClientJsonRpcMessage) -> Result<(), SessionError> {
        if self.to_service_tx.send(message).await.is_err() {
            return Err(SessionError::TransportClosed);
        }
        Ok(())
    }
    pub async fn establish_request_wise_channel(
        &mut self,
        request: ClientRequest,
        request_id: RequestId,
    ) -> Result<Receiver<ServerSessionMessage>, SessionError> {
        if self.request_router.contains_key(&request_id) {
            return Err(SessionError::DuplicatedRequestId(request_id.clone()));
        };
        let progress_token = request.get_meta().get_progress_token();
        let (tx, rx) = tokio::sync::mpsc::channel(Self::REQUEST_WISE_CHANNEL_SIZE);
        self.send_to_service(ClientJsonRpcMessage::Request(JsonRpcRequest {
            request,
            id: request_id.clone(),
            jsonrpc: crate::model::JsonRpcVersion2_0,
        }))
        .await?;
        self.request_router.insert(
            request_id.clone(),
            RequestWise {
                progress_token,
                tx: CachedTx::new(tx, Some(request_id)),
            },
        );
        Ok(rx)
    }
    fn resolve_outbound_channel(&self, message: &ServerJsonRpcMessage) -> OutboundChannel {
        match &message {
            ServerJsonRpcMessage::Request(_) => OutboundChannel::Common,
            ServerJsonRpcMessage::Notification(JsonRpcNotification {
                notification:
                    ServerNotification::ProgressNotification(Notification {
                        params: ProgressNotificationParam { progress_token, .. },
                        ..
                    }),
                ..
            }) => {
                let id = self.request_router.iter().find_map(|(id, r)| {
                    (r.progress_token.as_ref() == Some(progress_token)).then_some(id)
                });
                if let Some(id) = id {
                    OutboundChannel::RequestWise {
                        id: id.clone(),
                        close: false,
                    }
                } else {
                    OutboundChannel::Common
                }
            }
            ServerJsonRpcMessage::Notification(JsonRpcNotification {
                notification:
                    ServerNotification::CancelledNotification(Notification {
                        params: CancelledNotificationParam { request_id, .. },
                        ..
                    }),
                ..
            }) => OutboundChannel::RequestWise {
                id: request_id.clone(),
                close: false,
            },
            ServerJsonRpcMessage::Notification(_) => OutboundChannel::Common,
            ServerJsonRpcMessage::Response(json_rpc_response) => OutboundChannel::RequestWise {
                id: json_rpc_response.id.clone(),
                close: false,
            },
            ServerJsonRpcMessage::Error(json_rpc_error) => OutboundChannel::RequestWise {
                id: json_rpc_error.id.clone(),
                close: true,
            },
            ServerJsonRpcMessage::BatchRequest(_) | ServerJsonRpcMessage::BatchResponse(_) => {
                // the server side should never yield a batch request or response now
                unreachable!("server side won't yield batch request or response")
            }
        }
    }
    pub async fn handle_server_message(
        &mut self,
        message: ServerJsonRpcMessage,
    ) -> Result<(), SessionError> {
        let outbound_channel = self.resolve_outbound_channel(&message);
        match outbound_channel {
            OutboundChannel::RequestWise { id, close } => {
                let id = id.clone();
                if let Some(request_wise) = self.request_router.get_mut(&id) {
                    request_wise.tx.send(message).await;
                    if close {
                        self.request_router.remove(&id);
                    }
                } else {
                    return Err(SessionError::ChannelClosed(Some(id)));
                }
            }
            OutboundChannel::Common => self.common.send(message).await,
        }
        Ok(())
    }
    pub async fn resume(
        &mut self,
        last_event_id: EventId,
    ) -> Result<Receiver<ServerSessionMessage>, SessionError> {
        match last_event_id.request_id {
            Some(request_id) => {
                let request_wise = self
                    .request_router
                    .get_mut(&request_id)
                    .ok_or(SessionError::ChannelClosed(Some(request_id.clone())))?;
                let channel = tokio::sync::mpsc::channel(Self::REQUEST_WISE_CHANNEL_SIZE);
                let (tx, rx) = channel;
                request_wise.tx.tx = tx;
                let index = last_event_id.index;
                // sync messages after index
                request_wise.tx.sync(index).await?;
                Ok(rx)
            }
            None => {
                let channel = tokio::sync::mpsc::channel(Self::REQUEST_WISE_CHANNEL_SIZE);
                let (tx, rx) = channel;
                self.common.tx = tx;
                let index = last_event_id.index;
                // sync messages after index
                self.common.sync(index).await?;
                Ok(rx)
            }
        }
    }
}

enum SessionEvent {
    ServiceMessage(ServerJsonRpcMessage),
    ClientMessage(ClientJsonRpcMessage),
    EstablishRequestWiseChannel {
        request: ClientRequest,
        request_id: RequestId,
        responder: oneshot::Sender<Result<Receiver<ServerSessionMessage>, SessionError>>,
    },
    CloseRequestWiseChannel {
        id: RequestId,
        responder: oneshot::Sender<Result<(), SessionError>>,
    },
    Resume {
        last_event_id: EventId,
        responder: oneshot::Sender<Result<Receiver<ServerSessionMessage>, SessionError>>,
    },
}

#[derive(Debug, Clone)]
pub enum SessionQuitReason {
    ServiceTerminated,
    ClientTerminated,
}

impl SessionContext {
    pub async fn run_session(mut self: SessionContext) -> SessionQuitReason {
        let quit_reason = loop {
            let event = tokio::select! {
                event = self.event_rx.recv() => {
                    if let Some(event) = event {
                        event
                    } else {
                        break SessionQuitReason::ServiceTerminated;
                    }
                },
            };
            match event {
                SessionEvent::ServiceMessage(json_rpc_message) => {
                    let handle_result = self.handle_server_message(json_rpc_message).await;
                }
                SessionEvent::ClientMessage(json_rpc_message) => {
                    let handle_result = self.send_to_service(json_rpc_message).await;
                }
                SessionEvent::EstablishRequestWiseChannel {
                    request,
                    request_id,
                    responder,
                } => {
                    let handle_result = self
                        .establish_request_wise_channel(request, request_id)
                        .await;
                    let _ = responder.send(handle_result);
                }
                SessionEvent::CloseRequestWiseChannel { id, responder } => {
                    let handle_result = self.request_router.remove(&id);
                    let _ = responder.send(Ok(()));
                }
                SessionEvent::Resume {
                    last_event_id,
                    responder,
                } => {
                    let handle_result = self.resume(last_event_id).await;
                    let _ = responder.send(handle_result);
                }
            }
        };
        tracing::debug!("session terminated: {:?}", quit_reason);
        quit_reason
    }
}

pub struct Session {
    handle: SessionHandle,
    task_handle: tokio::task::JoinHandle<SessionQuitReason>,
}

impl Session {
    pub fn handle(&self) -> &SessionHandle {
        &self.handle
    }
}

#[derive(Debug, Clone)]
pub struct SessionHandle {
    event_tx: Sender<SessionEvent>,
}

impl SessionHandle {
    pub async fn push_message(&self, message: ClientJsonRpcMessage) -> Result<(), SessionError> {
        self.event_tx
            .send(SessionEvent::ClientMessage(message))
            .await
            .map_err(|_| SessionError::SessionServiceTerminated)?;
        Ok(())
    }

    pub async fn establish_request_wise_channel(
        &self,
        request: ClientRequest,
        request_id: RequestId,
    ) -> Result<Receiver<ServerSessionMessage>, SessionError> {
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.event_tx
            .send(SessionEvent::EstablishRequestWiseChannel {
                request,
                request_id,
                responder: tx,
            })
            .await
            .map_err(|_| SessionError::SessionServiceTerminated)?;
        rx.await
            .map_err(|_| SessionError::SessionServiceTerminated)?
    }
    pub async fn close_request_wise_channel(
        &self,
        request_id: RequestId,
    ) -> Result<(), SessionError> {
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.event_tx
            .send(SessionEvent::CloseRequestWiseChannel {
                id: request_id,
                responder: tx,
            })
            .await
            .map_err(|_| SessionError::SessionServiceTerminated)?;
        rx.await
            .map_err(|_| SessionError::SessionServiceTerminated)?
    }
    pub async fn establish_common_channel(
        &self,
    ) -> Result<Receiver<ServerSessionMessage>, SessionError> {
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.event_tx
            .send(SessionEvent::Resume {
                last_event_id: EventId {
                    request_id: None,
                    index: 0,
                },
                responder: tx,
            })
            .await
            .map_err(|_| SessionError::SessionServiceTerminated)?;
        rx.await
            .map_err(|_| SessionError::SessionServiceTerminated)?
    }

    pub async fn resume(
        &self,
        last_event_id: EventId,
    ) -> Result<Receiver<ServerSessionMessage>, SessionError> {
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.event_tx
            .send(SessionEvent::Resume {
                last_event_id,
                responder: tx,
            })
            .await
            .map_err(|_| SessionError::SessionServiceTerminated)?;
        rx.await
            .map_err(|_| SessionError::SessionServiceTerminated)?
    }
}

pub struct SessionTransport {
    from_service_rx: Receiver<ServerJsonRpcMessage>,
}

pub fn session(id: SessionId) -> (SessionContext, SessionTransport) {
    let (service_tx, service_rx) = tokio::sync::mpsc::channel(16);
    let (event_tx, event_rx) = tokio::sync::mpsc::channel(16);
    let (common_tx, _) = tokio::sync::mpsc::channel(0);
    let common = CachedTx::new_common(common_tx);
    let session_context = SessionContext {
        id,
        request_router: HashMap::new(),
        common,
        to_service_tx: service_tx,
        event_rx,
    };
    let session_transport = SessionTransport {
        from_service_rx: service_rx,
    };
    (session_context, session_transport)
}
