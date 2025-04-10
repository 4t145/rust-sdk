use std::{collections::HashMap, io, net::SocketAddr, sync::Arc, time::Duration};

use axum::{
    Json, Router,
    extract::{Query, State},
    http::{HeaderMap, HeaderValue, StatusCode},
    response::{
        IntoResponse, Response,
        sse::{Event, KeepAlive, Sse},
    },
    routing::{get, post},
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

use super::session::{Session, SessionId, SessionTransport};
type SessionManager = Arc<tokio::sync::RwLock<HashMap<SessionId, Session>>>;
pub type TransportReceiver = ReceiverStream<RxJsonRpcMessage<RoleServer>>;

const DEFAULT_AUTO_PING_INTERVAL: Duration = Duration::from_secs(15);

#[derive(Clone)]
struct App {
    session_manager: SessionManager,
    transport_tx: tokio::sync::mpsc::UnboundedSender<SessionTransport>,
    sse_ping_interval: Duration,
}

impl App {
    pub fn new(
        sse_ping_interval: Duration,
    ) -> (
        Self,
        tokio::sync::mpsc::UnboundedReceiver<SessionTransport>,
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
) -> Result<Response, StatusCode> {
    use futures::StreamExt;
    if let Some(session_id) = header_map.get(HEADER_SESSION_ID) {
        let session_id = session_id.to_str().map_err(|_| StatusCode::BAD_REQUEST)?;
        tracing::debug!(session_id, ?message, "new client message");
        let handle = {
            let sm = app.session_manager.read().await;
            let session = sm.get(session_id).ok_or(StatusCode::NOT_FOUND)?;
            session.handle().clone()
        };
        match &message {
            ClientJsonRpcMessage::Request(_) | ClientJsonRpcMessage::BatchRequest(_) => {
                let receiver = handle
                    .establish_request_wise_channel()
                    .await
                    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

                let stream =
                    ReceiverStream::new(receiver.inner).map(|message| match serde_json::to_string(
                        &message.message,
                    ) {
                        Ok(bytes) => Ok(Event::default()
                            .event("message")
                            .data(&bytes)
                            .id(message.event_id.to_string())),
                        Err(e) => Err(io::Error::new(io::ErrorKind::InvalidData, e)),
                    });
                return Ok(Sse::new(stream)
                    .keep_alive(KeepAlive::new().interval(app.sse_ping_interval))
                    .into_response());
            }
            _ => {
                let result = handle.push_message(message).await;
                if result.is_err() {
                    return Err(StatusCode::GONE);
                } else {
                    return Ok(StatusCode::ACCEPTED.into_response());
                }
            }
        }
    } else {
        // expect initialize message
        let session_id = session_id();
        let (session, transport) = super::session::session(session_id.clone());
        let Ok(_) = app.transport_tx.send(transport) else {
            return Err(StatusCode::GONE);
        };

        let Ok(response) = session.handle().initialize(message).await else {
            return Err(StatusCode::INTERNAL_SERVER_ERROR);
        };
        let mut response = Json(response).into_response();
        response.headers_mut().insert(
            HEADER_SESSION_ID,
            HeaderValue::from_bytes(session_id.as_bytes()).expect("should be valid header value"),
        );
        app.session_manager
            .write()
            .await
            .insert(session_id, session);
        return Ok(response);
    }
}

async fn get_handler(
    State(app): State<App>,
    header_map: HeaderMap,
) -> Result<Sse<impl Stream<Item = Result<Event, io::Error>>>, Response<String>> {
    let last_event_id = header_map
        .get("Last-Event-Id")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());
    
}

async fn delete_handler(
    State(app): State<App>,
    header_map: HeaderMap,
) -> Result<StatusCode, StatusCode> {
    if let Some(session_id) = header_map.get(HEADER_SESSION_ID) {
        let session_id = session_id.to_str().map_err(|_| StatusCode::BAD_REQUEST)?;
        let mut sm = app.session_manager.write().await;
        let session = sm.remove(session_id).ok_or(StatusCode::NOT_FOUND)?;
        session.handle();
        Ok(StatusCode::ACCEPTED)
    } else {
        Err(StatusCode::BAD_REQUEST)
    }
}

#[derive(Debug, Clone)]
pub struct SseServerConfig {
    pub bind: SocketAddr,
    pub sse_path: String,
    pub post_path: String,
    pub ct: CancellationToken,
    pub sse_keep_alive: Option<Duration>,
}
