use crate::error::Error as McpError;
use crate::schema::*;
use futures::{Sink, Stream};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum ServiceError {
    #[error("Mcp error: {0}")]
    McpError(McpError),
    #[error("Transport error: {0}")]
    Transport(std::io::Error),
}

impl ServiceError {
    pub fn into_mcp_error(self) -> McpError {
        match self {
            ServiceError::McpError(error) => error,
            ServiceError::Transport(error) => McpError::internal(error),
        }
    }
}

pub trait ServiceRole {
    type Req;
    type Resp;
    type Not;
    type PeerReq;
    type PeerResp;
    type PeerNot;
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RoleClient;
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RoleServer;
impl ServiceRole for RoleClient {
    type Req = ClientRequest;
    type Resp = ClientResult;
    type Not = ClientNotification;
    type PeerReq = ServerRequest;
    type PeerResp = ServerResult;
    type PeerNot = ServerNotification;
}

impl ServiceRole for RoleServer {
    type Req = ServerRequest;
    type Resp = ServerResult;
    type Not = ServerNotification;
    type PeerReq = ClientRequest;
    type PeerResp = ClientResult;
    type PeerNot = ClientNotification;
}

pub trait Service {
    type Role: ServiceRole;
    fn handle_request(
        &self,
        request: <Self::Role as ServiceRole>::PeerReq,
    ) -> impl Future<Output = Result<<Self::Role as ServiceRole>::Resp, ServiceError>> + '_;
    fn handle_notification(
        &self,
        notification: <Self::Role as ServiceRole>::PeerNot,
    ) -> impl Future<Output = Result<(), ServiceError>> + '_;
    fn send_request(
        &self,
        request: <Self::Role as ServiceRole>::Req,
    ) -> impl Future<Output = Result<<Self::Role as ServiceRole>::PeerResp, ServiceError>> + '_
    {
        async move {
            match self.get_peer_proxy() {
                Some(peer) => peer.send_request(request).await,
                None => Err(ServiceError::Transport(std::io::Error::other(
                    "peer proxy not set",
                ))),
            }
        }
    }
    fn send_notification(
        &self,
        notification: <Self::Role as ServiceRole>::Not,
    ) -> impl Future<Output = Result<(), ServiceError>> + '_ {
        async move {
            match self.get_peer_proxy() {
                Some(peer) => peer.send_notification(notification).await,
                None => Err(ServiceError::Transport(std::io::Error::other(
                    "peer proxy not set",
                ))),
            }
        }
    }
    fn get_peer_proxy(&self) -> Option<PeerProxy<Self::Role>>;
    fn set_peer_proxy(&mut self, peer: PeerProxy<Self::Role>);
}

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::AtomicU32;

use tokio::sync::mpsc;

pub trait RequestIdProvider: Send + Sync + 'static {
    fn next_request_id(&self) -> RequestId;
}
#[derive(Debug, Default)]
pub struct AtomicU32RequestIdProvider {
    id: AtomicU32,
}

impl RequestIdProvider for AtomicU32RequestIdProvider {
    fn next_request_id(&self) -> RequestId {
        RequestId::Number(self.id.fetch_add(1, std::sync::atomic::Ordering::SeqCst))
    }
}

type Responder<T> = tokio::sync::oneshot::Sender<T>;

pub enum PeerProxyMessage<R: ServiceRole> {
    Request(R::Req, RequestId, Responder<Result<R::PeerResp, ErrorData>>),
    Notification(R::Not),
}

#[derive(Clone)]
pub struct PeerProxy<R: ServiceRole> {
    tx: mpsc::Sender<PeerProxyMessage<R>>,
    request_id_provider: Arc<dyn RequestIdProvider>,
}

type ProxyOutbound<R> = mpsc::Receiver<PeerProxyMessage<R>>;
impl<R: ServiceRole> PeerProxy<R> {
    const CLIENT_CHANNEL_BUFFER_SIZE: usize = 1024;
    pub fn new(
        request_id_provider: Arc<dyn RequestIdProvider>,
    ) -> (PeerProxy<R>, ProxyOutbound<R>) {
        let (tx, rx) = mpsc::channel(Self::CLIENT_CHANNEL_BUFFER_SIZE);
        (
            Self {
                tx,
                request_id_provider,
            },
            rx,
        )
    }
    pub async fn send_notification(&self, notification: R::Not) -> Result<(), ServiceError> {
        self.tx
            .send(PeerProxyMessage::Notification(notification))
            .await
            .map_err(|_m| ServiceError::Transport(std::io::Error::other("disconnected")))
    }
    pub async fn send_request(&self, request: R::Req) -> Result<R::PeerResp, ServiceError> {
        let id = self.request_id_provider.next_request_id();
        let (responder, receiver) = tokio::sync::oneshot::channel();
        self.tx
            .send(PeerProxyMessage::Request(request, id, responder))
            .await
            .map_err(|_m| ServiceError::Transport(std::io::Error::other("disconnected")))?;
        let response = receiver
            .await
            .map_err(|_e| ServiceError::Transport(std::io::Error::other("disconnected")))?;
        let message = response.map_err(|e| ServiceError::McpError(e.into()))?;
        Ok(message)
    }
}

pub async fn serve<S, T, E>(mut service: S, transport: T) -> Result<(), E>
where
    S: Service,
    T: Stream<
            Item = JsonRpcMessage<
                <S::Role as ServiceRole>::PeerReq,
                <S::Role as ServiceRole>::PeerResp,
                <S::Role as ServiceRole>::PeerNot,
            >,
        > + Sink<
            JsonRpcMessage<
                <S::Role as ServiceRole>::Req,
                <S::Role as ServiceRole>::Resp,
                <S::Role as ServiceRole>::Not,
            >,
            Error = E,
        >,
    E: std::error::Error,
{
    use futures::{SinkExt, StreamExt};

    tracing::info!("Server started");
    let (mut sink, mut stream) = transport.split();
    let id_provider = Arc::new(AtomicU32RequestIdProvider::default());
    let (client, mut media) = <PeerProxy<S::Role>>::new(id_provider);
    service.set_peer_proxy(client);
    let mut local_responder_pool = HashMap::new();

    // let message_sink = tokio::sync::
    // let mut stream = std::pin::pin!(stream);
    enum Event<P, R> {
        ProxyMessage(P),
        RemoteMessage(R),
    }
    loop {
        let evt = tokio::select! {
            m = stream.next() => {
                if let Some(m) = m {
                    Event::RemoteMessage(m.into_message())
                } else {
                    continue
                }
            }
            m = media.recv() => {
                if let Some(m) = m {
                    Event::ProxyMessage(m)
                } else {
                    continue
                }
            }
        };
        match evt {
            Event::ProxyMessage(PeerProxyMessage::Request(request, id, responder)) => {
                local_responder_pool.insert(id.clone(), responder);
                sink.send(Message::Request(request, id).into_json_rpc_message())
                    .await?;
            }
            Event::ProxyMessage(PeerProxyMessage::Notification(message)) => {
                sink.send(Message::Notification(message).into_json_rpc_message())
                    .await?;
            }
            Event::RemoteMessage(Message::Request(request, id)) => {
                // Process the request using our service
                let response = match service.handle_request(request).await {
                    Ok(result) => Message::Response(result, id),
                    Err(e) => Message::Error(e.into_mcp_error().into(), id),
                }
                .into_json_rpc_message();

                // Send the response back
                sink.send(response).await?
            }
            Event::RemoteMessage(Message::Notification(notification)) => {
                // Serialize notification for logging
                let result = service.handle_notification(notification).await;
                if let Err(error) = result {
                    tracing::warn!(%error, "Error sending notification");
                }
            }
            Event::RemoteMessage(Message::Response(result, id)) => {
                if let Some(responder) = local_responder_pool.remove(&id) {
                    let response_result = responder.send(Ok(result));
                    if let Err(_error) = response_result {
                        tracing::warn!(%id, "Error sending response");
                    }
                }
            }
            Event::RemoteMessage(Message::Error(error, id)) => {
                if let Some(responder) = local_responder_pool.remove(&id) {
                    let _response_result = responder.send(Err(error));
                    if let Err(_error) = _response_result {
                        tracing::warn!(%id, "Error sending response");
                    }
                }
            }
            _ => {
                // invalid message
            }
        }
    }
    Ok(())
}
