use crate::error::Error as McpError;
use crate::schema::*;
use futures::{Sink, Stream};
use thiserror::Error;

mod client;
pub use client::*;
mod server;
pub use server::*;

#[derive(Error, Debug)]
pub enum ServiceError {
    #[error("Mcp error: {0}")]
    McpError(McpError),
    #[error("Transport error: {0}")]
    Transport(std::io::Error),
    #[error("Unexpected response type")]
    UnexpectedResponse,
}

impl ServiceError {}

pub trait ServiceRole: std::fmt::Debug + Send + Sync + 'static + Clone + Copy {
    type Req: std::fmt::Debug + Send + Sync + 'static;
    type Resp: std::fmt::Debug + Send + Sync + 'static;
    type Not: std::fmt::Debug + Send + Sync + 'static;
    type PeerReq: std::fmt::Debug + Send + Sync + 'static;
    type PeerResp: std::fmt::Debug + Send + Sync + 'static;
    type PeerNot: std::fmt::Debug + Send + Sync + 'static;
    const IS_CLIENT: bool;
    type Info: std::fmt::Debug + Send + Sync + 'static;
    type PeerInfo: std::fmt::Debug + Send + Sync + Clone + 'static;
}

pub trait Service: Send + Sync + 'static {
    type Role: ServiceRole;
    fn handle_request(
        &self,
        request: <Self::Role as ServiceRole>::PeerReq,
    ) -> impl Future<Output = Result<<Self::Role as ServiceRole>::Resp, McpError>> + Send + '_;
    fn handle_notification(
        &self,
        notification: <Self::Role as ServiceRole>::PeerNot,
    ) -> impl Future<Output = Result<(), McpError>> + Send + '_;
    fn get_peer(&self) -> Option<Peer<Self::Role>>;
    fn set_peer(&mut self, peer: Peer<Self::Role>);
    fn set_peer_info(&mut self, peer: <Self::Role as ServiceRole>::PeerInfo);
    fn get_peer_info(&self) -> Option<<Self::Role as ServiceRole>::PeerInfo>;
    fn get_info(&self) -> <Self::Role as ServiceRole>::Info;
}

use std::collections::HashMap;
use std::pin::Pin;
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
#[derive(Debug)]
pub enum PeerSinkMessage<R: ServiceRole> {
    Request(R::Req, RequestId, Responder<Result<R::PeerResp, ErrorData>>),
    Notification(R::Not),
}

#[derive(Clone)]
pub struct Peer<R: ServiceRole> {
    tx: mpsc::Sender<PeerSinkMessage<R>>,
    request_id_provider: Arc<dyn RequestIdProvider>,
    info: R::PeerInfo,
}

impl<R: ServiceRole> std::fmt::Debug for Peer<R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PeerSink")
            .field("tx", &self.tx)
            .field("is_client", &R::IS_CLIENT)
            .finish()
    }
}

type ProxyOutbound<R> = mpsc::Receiver<PeerSinkMessage<R>>;
impl<R: ServiceRole> Peer<R> {
    const CLIENT_CHANNEL_BUFFER_SIZE: usize = 1024;
    pub fn new(
        request_id_provider: Arc<dyn RequestIdProvider>,
        peer_info: R::PeerInfo,
    ) -> (Peer<R>, ProxyOutbound<R>) {
        let (tx, rx) = mpsc::channel(Self::CLIENT_CHANNEL_BUFFER_SIZE);
        (
            Self {
                tx,
                request_id_provider,
                info: peer_info,
            },
            rx,
        )
    }
    pub async fn send_notification(&self, notification: R::Not) -> Result<(), ServiceError> {
        self.tx
            .send(PeerSinkMessage::Notification(notification))
            .await
            .map_err(|_m| ServiceError::Transport(std::io::Error::other("disconnected")))
    }
    pub async fn send_request(&self, request: R::Req) -> Result<R::PeerResp, ServiceError> {
        let id = self.request_id_provider.next_request_id();
        let (responder, receiver) = tokio::sync::oneshot::channel();
        self.tx
            .send(PeerSinkMessage::Request(request, id, responder))
            .await
            .map_err(|_m| ServiceError::Transport(std::io::Error::other("disconnected")))?;
        let response = receiver
            .await
            .map_err(|_e| ServiceError::Transport(std::io::Error::other("disconnected")))?;
        let message = response.map_err(ServiceError::McpError)?;
        Ok(message)
    }
    pub fn info(&self) -> &R::PeerInfo {
        &self.info
    }
}

pub struct RunningService<S: Service, E> {
    service: Arc<S>,
    peer: Peer<S::Role>,
    handle: tokio::task::JoinHandle<Result<(), E>>,
}

impl<S: Service, E> RunningService<S, E> {
    #[inline]
    pub fn peer(&self) -> &Peer<S::Role> {
        &self.peer
    }
    #[inline]
    pub fn service(&self) -> &S {
        self.service.as_ref()
    }
    pub async fn waiting(self) -> Result<(), E> {
        self.handle.await.expect("tokio join error")
    }
}

async fn serve_inner<S, T, E>(
    mut service: S,
    mut transport: T,
    initial_hook: impl AsyncFnOnce(
        &mut S,
        &mut T,
        &Arc<AtomicU32RequestIdProvider>,
    ) -> Result<<S::Role as ServiceRole>::PeerInfo, E>
    + Send,
) -> Result<RunningService<S, E>, E>
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
        > + Send
        + Unpin
        + 'static,
    E: Send + 'static,
{
    use futures::{SinkExt, StreamExt};
    const SINK_PROXY_BUFFER_SIZE: usize = 1024;
    tracing::info!("Server started");
    let (sink_proxy_tx, mut sink_proxy_rx) = tokio::sync::mpsc::channel::<
        JsonRpcMessage<
            <S::Role as ServiceRole>::Req,
            <S::Role as ServiceRole>::Resp,
            <S::Role as ServiceRole>::Not,
        >,
    >(SINK_PROXY_BUFFER_SIZE);
    let id_provider = Arc::new(AtomicU32RequestIdProvider::default());
    // call initialize hook
    let peer_info = initial_hook(&mut service, &mut transport, &id_provider).await?;

    if S::Role::IS_CLIENT {
        tracing::info!("Initialized as client");
    } else {
        tracing::info!("Initialized as server");
    }

    let (peer, mut peer_proxy) = <Peer<S::Role>>::new(id_provider, peer_info);
    service.set_peer(peer.clone());
    let mut local_responder_pool = HashMap::new();
    let shared_service = Arc::new(service);
    // for return
    let service = shared_service.clone();

    // let message_sink = tokio::sync::
    // let mut stream = std::pin::pin!(stream);
    let (mut sink, mut stream) = transport.split();
    let handle = tokio::spawn(async move {
        #[derive(Debug)]
        enum Event<P, R, T> {
            ProxyMessage(P),
            RemoteMessage(R),
            ToSink(T),
        }
        loop {
            let evt = tokio::select! {
                m = sink_proxy_rx.recv() => {
                    if let Some(m) = m {
                        Event::ToSink(m)
                    } else {
                        continue
                    }
                }
                m = stream.next() => {
                    if let Some(m) = m {
                        Event::RemoteMessage(m.into_message())
                    } else {
                        // input stream closed
                        break
                    }
                }
                m = peer_proxy.recv() => {
                    if let Some(m) = m {
                        Event::ProxyMessage(m)
                    } else {
                        continue
                    }
                }
            };
            tracing::debug!(?evt, "new event");
            match evt {
                Event::ToSink(e) => {
                    sink.send(e).await?;
                }
                Event::ProxyMessage(PeerSinkMessage::Request(request, id, responder)) => {
                    local_responder_pool.insert(id.clone(), responder);
                    sink.send(Message::Request(request, id).into_json_rpc_message())
                        .await?;
                }
                Event::ProxyMessage(PeerSinkMessage::Notification(message)) => {
                    sink.send(Message::Notification(message).into_json_rpc_message())
                        .await?;
                }
                Event::RemoteMessage(Message::Request(request, id)) => {
                    tracing::info!(%id, ?request, "received request");
                    {
                        let service = shared_service.clone();
                        let sink = sink_proxy_tx.clone();
                        tokio::spawn(async move {
                            let result = service.handle_request(request).await;
                            let response = match result {
                                Ok(result) => {
                                    tracing::info!(%id, ?result, "response message");
                                    Message::Response(result, id)
                                }
                                Err(error) => {
                                    tracing::warn!(%id, ?error, "response error");
                                    Message::Error(error, id)
                                }
                            }
                            .into_json_rpc_message();
                            let send_result = sink.send(response).await;
                        });
                    }
                }
                Event::RemoteMessage(Message::Notification(notification)) => {
                    tracing::info!(?notification, "received notification");
                    {
                        let service = shared_service.clone();
                        tokio::spawn(async move {
                            let result = service.handle_notification(notification).await;
                            if let Err(error) = result {
                                tracing::warn!(%error, "Error sending notification");
                            }
                        });
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
    });
    Ok(RunningService {
        service,
        peer,
        handle,
    })
}
