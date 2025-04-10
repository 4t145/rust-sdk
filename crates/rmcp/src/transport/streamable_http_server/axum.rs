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

use super::session::{
    self, EventId, Session, SessionId, SessionTransport, StreamableHttpMessageReceiver,
};
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
    ) -> (Self, tokio::sync::mpsc::UnboundedReceiver<SessionTransport>) {
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

fn receiver_as_stream(
    receiver: StreamableHttpMessageReceiver,
) -> impl Stream<Item = Result<Event, io::Error>> {
    use futures::StreamExt;
    ReceiverStream::new(receiver.inner).map(|message| {
        match serde_json::to_string(&message.message) {
            Ok(bytes) => Ok(Event::default()
                .event("message")
                .data(&bytes)
                .id(message.event_id.to_string())),
            Err(e) => Err(io::Error::new(io::ErrorKind::InvalidData, e)),
        }
    })
}

async fn post_handler(
    State(app): State<App>,
    header_map: HeaderMap,
    Json(message): Json<ClientJsonRpcMessage>,
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
                Ok(Sse::new(stream)
                    .keep_alive(KeepAlive::new().interval(app.sse_ping_interval))
                    .into_response())
            }
            _ => {
                let result = handle.push_message(message).await;
                if result.is_err() {
                    Err(StatusCode::GONE)
                } else {
                    Ok(StatusCode::ACCEPTED.into_response())
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
) -> Result<Sse<impl Stream<Item = Result<Event, io::Error>>>, Response> {
    let session_id = header_map
        .get(HEADER_SESSION_ID)
        .and_then(|v| v.to_str().ok());
    if let Some(session_id) = session_id {
        let last_event_id = header_map
            .get("Last-Event-Id")
            .and_then(|v| v.to_str().ok());
        match last_event_id {
            Some(last_event_id) => {
                let last_event_id = last_event_id.parse::<EventId>().map_err(|e| {
                    (StatusCode::BAD_REQUEST, format!("invalid event_id {e}")).into_response()
                })?;
                let sm = app.session_manager.read().await;
                let session = sm.get(session_id).ok_or_else(|| {
                    (
                        StatusCode::NOT_FOUND,
                        format!("session {session_id} not found"),
                    )
                        .into_response()
                })?;
                let handle = session.handle();
                let receiver = handle.resume(last_event_id).await.map_err(|e| {
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        format!("resume error {e}"),
                    )
                        .into_response()
                })?;
                let stream = receiver_as_stream(receiver);
                return Ok(
                    Sse::new(stream).keep_alive(KeepAlive::new().interval(app.sse_ping_interval))
                );
            }
            None => {
                let sm = app.session_manager.read().await;
                let session = sm.get(session_id).ok_or_else(|| {
                    (
                        StatusCode::NOT_FOUND,
                        format!("session {session_id} not found"),
                    )
                        .into_response()
                })?;
                let handle = session.handle();
                let receiver = handle.establish_common_channel().await.map_err(|e| {
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        format!("establish common channel error {e}"),
                    )
                        .into_response()
                })?;
                let stream = receiver_as_stream(receiver);
                return Ok(
                    Sse::new(stream).keep_alive(KeepAlive::new().interval(app.sse_ping_interval))
                );
            }
        }
    } else {
        Err((StatusCode::BAD_REQUEST, "missing session id").into_response())
    }
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
    pub path: String,
    pub ct: CancellationToken,
    pub sse_keep_alive: Option<Duration>,
}

#[derive(Debug)]
pub struct StreamableHttpServer {
    transport_rx: tokio::sync::mpsc::UnboundedReceiver<SessionTransport>,
    pub config: SseServerConfig,
}

impl StreamableHttpServer {
    pub async fn serve(bind: SocketAddr) -> io::Result<Self> {
        Self::serve_with_config(SseServerConfig {
            bind,
            path: "/sse".to_string(),
            ct: CancellationToken::new(),
            sse_keep_alive: None,
        })
        .await
    }
    pub async fn serve_with_config(config: SseServerConfig) -> io::Result<Self> {
        let (sse_server, service) = Self::new(config);
        let listener = tokio::net::TcpListener::bind(sse_server.config.bind).await?;
        let ct = sse_server.config.ct.child_token();
        let server = axum::serve(listener, service).with_graceful_shutdown(async move {
            ct.cancelled().await;
            tracing::info!("sse server cancelled");
        });
        tokio::spawn(
            async move {
                if let Err(e) = server.await {
                    tracing::error!(error = %e, "sse server shutdown with error");
                }
            }
            .instrument(tracing::info_span!("sse-server", bind_address = %sse_server.config.bind)),
        );
        Ok(sse_server)
    }

    /// Warning: This function creates a new SseServer instance with the provided configuration.
    /// `App.post_path` may be incorrect if using `Router` as an embedded router.
    pub fn new(config: SseServerConfig) -> (StreamableHttpServer, Router) {
        let (app, transport_rx) =
            App::new(config.sse_keep_alive.unwrap_or(DEFAULT_AUTO_PING_INTERVAL));
        let router = Router::new()
            .route(
                &config.path,
                get(get_handler).post(post_handler).delete(delete_handler),
            )
            .with_state(app);

        let server = StreamableHttpServer {
            transport_rx,
            config,
        };

        (server, router)
    }

    pub fn with_service<S, F>(mut self, service_provider: F) -> CancellationToken
    where
        S: Service<RoleServer>,
        F: Fn() -> S + Send + 'static,
    {
        use crate::service::ServiceExt;
        let ct = self.config.ct.clone();
        tokio::spawn(async move {
            while let Some(transport) = self.next_transport().await {
                let service = service_provider();
                let ct = self.config.ct.child_token();
                tokio::spawn(async move {
                    let server = service.serve_with_ct(transport, ct).await?;
                    server.waiting().await?;
                    tokio::io::Result::Ok(())
                });
            }
        });
        ct
    }

    pub fn cancel(&self) {
        self.config.ct.cancel();
    }

    pub async fn next_transport(&mut self) -> Option<SessionTransport> {
        self.transport_rx.recv().await
    }
}
