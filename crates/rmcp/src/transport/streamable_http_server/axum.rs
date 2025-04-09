use std::{collections::HashMap, io, net::SocketAddr, sync::Arc, time::Duration};

use axum::{
    extract::{Query, State}, http::{HeaderMap, StatusCode}, response::{
        sse::{Event, KeepAlive, Sse}, IntoResponse, Response
    }, routing::{get, post}, Json, Router
};
use futures::{Sink, SinkExt, Stream};
use tokio_stream::wrappers::ReceiverStream;
use tokio_util::sync::{CancellationToken, PollSender};
use tracing::Instrument;

use crate::{
    RoleServer, Service,
    model::ClientJsonRpcMessage,
    service::{RxJsonRpcMessage, TxJsonRpcMessage},
    transport::streamable_http_server::session::HEADER_SESSION_ID,
};

use super::session::{Session, SessionId};
type SessionManager = Arc<tokio::sync::RwLock<HashMap<SessionId, Session>>>;
pub type TransportReceiver = ReceiverStream<RxJsonRpcMessage<RoleServer>>;

const DEFAULT_AUTO_PING_INTERVAL: Duration = Duration::from_secs(15);

#[derive(Clone)]
struct App {
    session_manager: SessionManager,
    transport_tx: tokio::sync::mpsc::UnboundedSender<SseServerTransport>,
    sse_ping_interval: Duration,
}

impl App {
    pub fn new(
        post_path: String,
        sse_ping_interval: Duration,
    ) -> (
        Self,
        tokio::sync::mpsc::UnboundedReceiver<SseServerTransport>,
    ) {
        let (transport_tx, transport_rx) = tokio::sync::mpsc::unbounded_channel();
        (
            Self {
                session_manager: Default::default(),
                transport_tx,
                sse_ping_interval,
            },
            transport_rx,
        )
    }
}

fn session_id() -> SessionId {
    let id = format!("{:016x}", rand::random::<u128>());
    Arc::from(id)
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PostEventQuery {
    pub session_id: String,
}

async fn post_handler(
    State(app): State<App>,
    Json(message): Json<ClientJsonRpcMessage>,
    header_map: HeaderMap,
) -> Result<impl IntoResponse, StatusCode> {
    if let Some(session_id) = header_map.get(HEADER_SESSION_ID) {
        let session_id = session_id.to_str().map_err(|_| StatusCode::BAD_REQUEST)?;
        tracing::debug!(session_id, ?message, "new client message");
        let handle = {
            let session = app
                .session_manager
                .read()
                .await
                .get(session_id)
                .ok_or(StatusCode::NOT_FOUND)?;
            session.handle().clone()
        };
        match &message {
            ClientJsonRpcMessage::Request(_) | ClientJsonRpcMessage::BatchRequest(_) => {
                let receiver = handle
                    .establish_request_wise_channel()
                    .await
                    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
                let stream = futures::stream::once(futures::future::ok(
                    Event::default()
                        .event("endpoint")
                        .data(format!("?sessionId={session}")),
                ))
                .chain(ReceiverStream::new(to_client_rx).map(|message| {
                    match serde_json::to_string(&message) {
                        Ok(bytes) => Ok(Event::default().event("message").data(&bytes)),
                        Err(e) => Err(io::Error::new(io::ErrorKind::InvalidData, e)),
                    }
                }))
            }
            ClientJsonRpcMessage::Notification(json_rpc_notification) => todo!(),
            ClientJsonRpcMessage::Response(json_rpc_response) => todo!(),
            ClientJsonRpcMessage::BatchResponse(json_rpc_batch_response_items) => todo!(),
            ClientJsonRpcMessage::Error(json_rpc_error) => todo!(),
        }
        handle
            .push_message(message)
            .await
            .map_err(|_| StatusCode::GONE)?;
    } else {
    };
    let tx = {
        let rg = app.session_manager.read().await;
        rg.get(session_id.as_str())
            .ok_or(StatusCode::NOT_FOUND)?
            .clone()
    };
    if tx.send(message).await.is_err() {
        tracing::error!("send message error");
        return Err(StatusCode::GONE);
    }
    Ok(StatusCode::ACCEPTED)
}

async fn get_handler(
    State(app): State<App>,
) -> Result<Sse<impl Stream<Item = Result<Event, io::Error>>>, Response<String>> {
    let session = session_id();
    tracing::info!(%session, "sse connection");
    use tokio_stream::{StreamExt, wrappers::ReceiverStream};
    use tokio_util::sync::PollSender;
    let (from_client_tx, from_client_rx) = tokio::sync::mpsc::channel(64);
    let (to_client_tx, to_client_rx) = tokio::sync::mpsc::channel(64);
    app.session_manager
        .write()
        .await
        .insert(session.clone(), from_client_tx);
    let session = session.clone();
    let stream = ReceiverStream::new(from_client_rx);
    let sink = PollSender::new(to_client_tx);
    let transport = SseServerTransport {
        stream,
        sink,
        session_id: session.clone(),
        tx_store: app.session_manager.clone(),
    };
    let transport_send_result = app.transport_tx.send(transport);
    if transport_send_result.is_err() {
        tracing::warn!("send transport out error");
        let mut response =
            Response::new("fail to send out transport, it seems server is closed".to_string());
        *response.status_mut() = StatusCode::INTERNAL_SERVER_ERROR;
        return Err(response);
    }
    let ping_interval = app.sse_ping_interval;
    let stream = futures::stream::once(futures::future::ok(
        Event::default()
            .event("endpoint")
            .data(format!("?sessionId={session}")),
    ))
    .chain(ReceiverStream::new(to_client_rx).map(|message| {
        match serde_json::to_string(&message) {
            Ok(bytes) => Ok(Event::default().event("message").data(&bytes)),
            Err(e) => Err(io::Error::new(io::ErrorKind::InvalidData, e)),
        }
    }));
    Ok(Sse::new(stream).keep_alive(KeepAlive::new().interval(ping_interval)))
}

async fn delete_handler(
    State(app): State<App>,
) -> Result<Sse<impl Stream<Item = Result<Event, io::Error>>>, Response<String>> {
    let session = session_id();
    tracing::info!(%session, "sse connection");
    use tokio_stream::{StreamExt, wrappers::ReceiverStream};
    use tokio_util::sync::PollSender;
    let (from_client_tx, from_client_rx) = tokio::sync::mpsc::channel(64);
    let (to_client_tx, to_client_rx) = tokio::sync::mpsc::channel(64);
    app.session_manager
        .write()
        .await
        .insert(session.clone(), from_client_tx);
    let session = session.clone();
    let stream = ReceiverStream::new(from_client_rx);
    let sink = PollSender::new(to_client_tx);
    let transport = SseServerTransport {
        stream,
        sink,
        session_id: session.clone(),
        tx_store: app.session_manager.clone(),
    };
    let transport_send_result = app.transport_tx.send(transport);
    if transport_send_result.is_err() {
        tracing::warn!("send transport out error");
        let mut response =
            Response::new("fail to send out transport, it seems server is closed".to_string());
        *response.status_mut() = StatusCode::INTERNAL_SERVER_ERROR;
        return Err(response);
    }
    let ping_interval = app.sse_ping_interval;
    let stream = futures::stream::once(futures::future::ok(
        Event::default()
            .event("endpoint")
            .data(format!("?sessionId={session}")),
    ))
    .chain(ReceiverStream::new(to_client_rx).map(|message| {
        match serde_json::to_string(&message) {
            Ok(bytes) => Ok(Event::default().event("message").data(&bytes)),
            Err(e) => Err(io::Error::new(io::ErrorKind::InvalidData, e)),
        }
    }));
    Ok(Sse::new(stream).keep_alive(KeepAlive::new().interval(ping_interval)))
}

#[derive(Debug, Clone)]
pub struct SseServerConfig {
    pub bind: SocketAddr,
    pub sse_path: String,
    pub post_path: String,
    pub ct: CancellationToken,
    pub sse_keep_alive: Option<Duration>,
}
