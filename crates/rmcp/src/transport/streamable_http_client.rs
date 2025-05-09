use std::{borrow::Cow, sync::Arc, time::Duration};

use futures::{Sink, SinkExt, Stream, StreamExt, stream::BoxStream};
pub use sse_stream::Error as SseError;
use sse_stream::Sse;
use thiserror::Error;
use tokio_stream::wrappers::ReceiverStream;
use tokio_util::sync::{CancellationToken, PollSender};
use tracing::Instrument;

use crate::model::{ClientJsonRpcMessage, ServerJsonRpcMessage};

type BoxedSseStream = BoxStream<'static, Result<Sse, SseError>>;

#[derive(Error, Debug)]
pub enum StreamableHttpError<E: std::error::Error + Send + Sync + 'static> {
    #[error("SSE error: {0}")]
    Sse(#[from] SseError),
    #[error("Io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Client error: {0}")]
    Client(E),
    #[error("unexpected end of stream")]
    UnexpectedEndOfStream,
    #[error("unexpected server response: {0}")]
    UnexpectedServerResponse(Cow<'static, str>),
    #[error("Unexpected content type: {0:?}")]
    UnexpectedContentType(Option<String>),
    #[error("Server does not support SSE")]
    SeverDoesNotSupportSse,
    #[error("Server does not support delete session")]
    SeverDoesNotSupportDeleteSession,
    #[error("Tokio join error: {0}")]
    TokioJoinError(#[from] tokio::task::JoinError),
    #[error("Deserialize error: {0}")]
    Deserialize(#[from] serde_json::Error),
    #[error("Transport channel closed")]
    TransportChannelClosed,
}

#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq)]
pub struct SseRetryConfig {
    pub max_times: Option<usize>,
    pub min_duration: Duration,
}

impl SseRetryConfig {
    pub const DEFAULT_MIN_DURATION: Duration = Duration::from_millis(1000);
}

impl Default for SseRetryConfig {
    fn default() -> Self {
        Self {
            max_times: None,
            min_duration: Self::DEFAULT_MIN_DURATION,
        }
    }
}

impl From<reqwest::Error> for StreamableHttpError<reqwest::Error> {
    fn from(e: reqwest::Error) -> Self {
        StreamableHttpError::Client(e)
    }
}

pub enum StreamableHttpPostResponse {
    Accepted,
    Json(StreamableHttpPostJsonResponse),
    Sse(BoxedSseStream),
}

pub struct StreamableHttpPostJsonResponse {
    pub message: ServerJsonRpcMessage,
    pub session_id: Option<String>,
}

impl StreamableHttpPostResponse {
    pub fn expect_json<E>(self) -> Result<StreamableHttpPostJsonResponse, StreamableHttpError<E>>
    where
        E: std::error::Error + Send + Sync + 'static,
    {
        match self {
            Self::Json(message) => Ok(message),
            _ => Err(StreamableHttpError::UnexpectedServerResponse(
                "expected json".into(),
            )),
        }
    }

    pub fn expect_accepted<E>(self) -> Result<(), StreamableHttpError<E>>
    where
        E: std::error::Error + Send + Sync + 'static,
    {
        match self {
            Self::Accepted => Ok(()),
            _ => Err(StreamableHttpError::UnexpectedServerResponse(
                "expected accepted".into(),
            )),
        }
    }
}

pub trait StreamableHttpClient: Clone + Send + 'static {
    type Error: std::error::Error + Send + Sync + 'static;
    fn post_message(
        &self,
        uri: Arc<str>,
        message: ClientJsonRpcMessage,
        session_id: Option<Arc<str>>,
    ) -> impl Future<Output = Result<StreamableHttpPostResponse, StreamableHttpError<Self::Error>>>
    + Send
    + '_;
    fn delete_session(
        &self,
        uri: Arc<str>,
        session_id: Arc<str>,
    ) -> impl Future<Output = Result<(), StreamableHttpError<Self::Error>>> + Send + '_;
    fn get_stream(
        &self,
        uri: Arc<str>,
        session_id: Arc<str>,
        last_event_id: Option<String>,
    ) -> impl Future<
        Output = Result<
            BoxStream<'static, Result<Sse, SseError>>,
            StreamableHttpError<Self::Error>,
        >,
    > + Send
    + '_;
}

pub struct RetryConfig {
    pub max_times: Option<usize>,
    pub min_duration: Duration,
}

/// # Transport for client sse
///
/// Call [`SseTransport::start`] to create a  new transport from url.
///
/// Call [`SseTransport::start_with_client`] to create a new transport with a customized reqwest client.
pub struct StreamableHttpClientTransport<C: StreamableHttpClient> {
    rx: ReceiverStream<ServerJsonRpcMessage>,
    tx: PollSender<ClientJsonRpcMessage>,
    result: Arc<std::sync::OnceLock<Result<(), StreamableHttpError<C::Error>>>>,
    _join_handle: tokio::task::JoinHandle<()>,
    _drop_guard: tokio_util::sync::DropGuard,
}

#[derive(Debug, Clone)]
pub struct StreamableHttpClientTransportConfig {
    pub uri: Arc<str>,
    pub retry_config: SseRetryConfig,
    pub channel_buffer_capacity: usize,
}

impl Default for StreamableHttpClientTransportConfig {
    fn default() -> Self {
        Self {
            uri: "localhost".into(),
            retry_config: SseRetryConfig::default(),
            channel_buffer_capacity: 16,
        }
    }
}

impl<C: StreamableHttpClient> StreamableHttpClientTransport<C> {
    fn try_get_result(&mut self) -> Result<(), StreamableHttpError<C::Error>> {
        if let Some(x) = Arc::get_mut(&mut self.result) {
            if let Some(result) = x.take() {
                return result;
            }
        }
        Err(StreamableHttpError::TransportChannelClosed)
    }
    async fn execute_sse_stream(
        client: C,
        sse_stream: BoxedSseStream,
        sse_worker_tx: tokio::sync::mpsc::Sender<ServerJsonRpcMessage>,
        session_id: Arc<str>,
        config: StreamableHttpClientTransportConfig,
        ct: CancellationToken,
    ) -> Result<(), StreamableHttpError<C::Error>> {
        let mut sse_stream = sse_stream;
        let mut retry_interval = config.retry_config.min_duration;
        let mut last_event_id = None;
        loop {
            let event = tokio::select! {
                event = sse_stream.next() => {
                    event
                }
                _ = ct.cancelled() => {
                    tracing::debug!("cancelled");
                    break;
                }
            };
            let next_sse = match event {
                Some(Ok(next_sse)) => next_sse,
                Some(Err(e)) => {
                    tracing::warn!("sse stream error: {e}");
                    let mut retry_times = 0;
                    'retry_loop: loop {
                        tracing::debug!("sse stream error: {e}, retrying in {:?}", retry_interval);
                        tokio::time::sleep(retry_interval).await;
                        let retry_result = client
                            .get_stream(config.uri.clone(), session_id.clone(), last_event_id.clone())
                            .await;
                        retry_times += 1;
                        match retry_result {
                            Ok(new_stream) => {
                                sse_stream = new_stream;
                                break 'retry_loop;
                            }
                            Err(e) => {
                                if retry_times
                                    >= config.retry_config.max_times.unwrap_or(usize::MAX)
                                {
                                    tracing::error!(
                                        "sse stream error: {e}, max retry times reached"
                                    );
                                    return Err(e);
                                } else {
                                    continue 'retry_loop;
                                }
                            }
                        }
                    }
                    continue;
                }
                None => {
                    tracing::debug!("sse stream terminated");
                    break;
                }
            };
            // set the retry interval
            if let Some(server_retry_interval) = next_sse.retry {
                retry_interval = retry_interval.min(Duration::from_millis(server_retry_interval));
            }

            if let Some(data) = next_sse.data {
                match serde_json::from_slice::<ServerJsonRpcMessage>(data.as_bytes()) {
                    Err(e) => tracing::warn!("failed to deserialize server message: {e}"),
                    Ok(message) => {
                        let yeild_result = sse_worker_tx.send(message).await;
                        if yeild_result.is_err() {
                            tracing::trace!("streamable http transport worker dropped, exiting");
                            break;
                        }
                    }
                };
            }

            if let Some(id) = next_sse.id {
                last_event_id = Some(id);
            }
        }
        Ok(())
    }
    pub fn start_with_client(client: C, config: StreamableHttpClientTransportConfig) -> Self {
        let transport_task_ct = CancellationToken::new();
        let (to_transport_tx, mut from_handler_rx) =
            tokio::sync::mpsc::channel::<ClientJsonRpcMessage>(config.channel_buffer_capacity);
        let (to_handler_tx, from_transport_rx) =
            tokio::sync::mpsc::channel::<ServerJsonRpcMessage>(config.channel_buffer_capacity);
        let (sse_worker_tx, mut sse_worker_rx) =
            tokio::sync::mpsc::channel::<ServerJsonRpcMessage>(config.channel_buffer_capacity);
        let result_once_lock = Arc::new(std::sync::OnceLock::new());
        let join_handle = tokio::spawn({
            let transport_task_ct = transport_task_ct.clone();
            let final_ct = transport_task_ct.clone();
            let result_once_lock = result_once_lock.clone();
            let task = async move {
                let initialize_request = from_handler_rx
                    .recv()
                    .await
                    .ok_or(StreamableHttpError::UnexpectedEndOfStream)?;
                let StreamableHttpPostJsonResponse {
                    session_id,
                    message,
                } = client
                    .post_message(config.uri.clone(), initialize_request, None)
                    .await?
                    .expect_json()?;
                let Some(session_id) = session_id else {
                    return Err(StreamableHttpError::UnexpectedServerResponse(
                        "missing session id".into(),
                    ));
                };
                let session_id: Arc<str> = session_id.into();
                to_handler_tx
                    .send(message)
                    .await
                    .map_err(|_| StreamableHttpError::UnexpectedEndOfStream)?;
                let initialized_notification = from_handler_rx
                    .recv()
                    .await
                    .ok_or(StreamableHttpError::UnexpectedEndOfStream)?;
                // expect a initialized response
                client
                    .post_message(config.uri.clone(), initialized_notification, Some(session_id.clone()))
                    .await?
                    .expect_accepted()?;

                enum Event<E: std::error::Error + Send + Sync + 'static> {
                    ClientMessage(ClientJsonRpcMessage),
                    ServerMessage(ServerJsonRpcMessage),
                    StreamResult(Result<(), StreamableHttpError<E>>),
                }
                let mut streams = tokio::task::JoinSet::new();
                match client.get_stream(config.uri.clone(), session_id.clone(), None).await {
                    Ok(stream) => {
                        streams.spawn(Self::execute_sse_stream(
                            client.clone(),
                            stream,
                            sse_worker_tx.clone(),
                            session_id.clone(),
                            config.clone(),
                            transport_task_ct.child_token(),
                        ));
                        tracing::debug!("got common stream");
                    }
                    Err(StreamableHttpError::SeverDoesNotSupportSse) => {}
                    Err(e) => {
                        // fail to get common stream
                        tracing::error!("fail to get common stream: {e}");
                        return Err(e);
                    }
                }

                loop {
                    let event: Event<C::Error> = tokio::select! {
                        message = from_handler_rx.recv() => {
                            let Some(message) = message else {
                                tracing::trace!("transport dropped, exiting");
                                break;
                            };
                            Event::ClientMessage(message)
                        },
                        message = sse_worker_rx.recv() => {
                            let Some(message) = message else {
                                tracing::trace!("transport dropped, exiting");
                                break;
                            };
                            Event::ServerMessage(message)
                        },
                        terminated_stream = streams.join_next() => {
                            match terminated_stream {
                                Some(result) => {
                                    Event::StreamResult(result.map_err(StreamableHttpError::TokioJoinError).and_then(std::convert::identity))
                                }
                                None => {
                                    continue
                                }
                            }
                        }
                    };
                    match event {
                        Event::ClientMessage(json_rpc_message) => {
                            let response = client
                                .post_message(config.uri.clone(), json_rpc_message, Some(session_id.clone()))
                                .await?;
                            match response {
                                StreamableHttpPostResponse::Accepted => {
                                    tracing::trace!("client message accepted");
                                }
                                StreamableHttpPostResponse::Json(message) => {
                                    to_handler_tx
                                        .send(message.message)
                                        .await
                                        .map_err(|_| StreamableHttpError::UnexpectedEndOfStream)?;
                                }
                                StreamableHttpPostResponse::Sse(stream) => {
                                    streams.spawn(Self::execute_sse_stream(
                                        client.clone(),
                                        stream,
                                        sse_worker_tx.clone(),
                                        session_id.clone(),
                                        config.clone(),
                                        transport_task_ct.child_token(),
                                    ));
                                    tracing::trace!("got new sse stream");
                                }
                            }
                        }
                        Event::ServerMessage(json_rpc_message) => {
                            // send the message to the handler
                            let send_result = to_handler_tx.send(json_rpc_message).await;
                            if send_result.is_err() {
                                tracing::trace!("transport dropped, exiting");
                                break;
                            }
                        }
                        Event::StreamResult(result) => {
                            if result.is_err() {
                                tracing::warn!(
                                    "sse client event stream terminated with error: {:?}",
                                    result
                                );
                            }
                        }
                    }
                }
                transport_task_ct.cancel();
                streams.join_all().await;
                let delete_session_result = client.delete_session(config.uri.clone(), session_id.clone()).await;
                match delete_session_result {
                    Ok(_) => {
                        tracing::info!(session_id = session_id.as_ref(), "delete session success")
                    }
                    Err(StreamableHttpError::SeverDoesNotSupportDeleteSession) => {
                        tracing::info!(
                            session_id = session_id.as_ref(),
                            "server doesn't support delete session"
                        )
                    }
                    Err(e) => {
                        tracing::error!(
                            session_id = session_id.as_ref(),
                            "fail to delete session: {e}"
                        );
                        return Err(e);
                    }
                }
                Result::<(), StreamableHttpError<C::Error>>::Ok(())
            }.instrument(tracing::info_span!("sse_transport_task"));
            async move {
                let result = task.await.inspect_err(|e| {
                    tracing::warn!(error = ?e, "sse transport task finished with error");
                });
                result_once_lock.get_or_init(|| result);
                drop(result_once_lock);
                final_ct.cancel();
            }
        });
        Self {
            rx: ReceiverStream::new(from_transport_rx),
            tx: PollSender::new(to_transport_tx),
            _join_handle: join_handle,
            _drop_guard: transport_task_ct.drop_guard(),
            result: result_once_lock,
        }
    }
}

impl<C> Stream for StreamableHttpClientTransport<C>
where
    C: StreamableHttpClient,
{
    type Item = ServerJsonRpcMessage;

    fn poll_next(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        self.rx.poll_next_unpin(cx)
    }
}

impl<C> Sink<ClientJsonRpcMessage> for StreamableHttpClientTransport<C>
where
    C: StreamableHttpClient,
{
    type Error = StreamableHttpError<C::Error>;

    fn poll_ready(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), Self::Error>> {
        let result = std::task::ready!(self.tx.poll_ready_unpin(cx));
        match result {
            Ok(()) => std::task::Poll::Ready(Ok(())),
            Err(_) => std::task::Poll::Ready(self.try_get_result()),
        }
    }

    fn start_send(
        mut self: std::pin::Pin<&mut Self>,
        item: ClientJsonRpcMessage,
    ) -> Result<(), Self::Error> {
        let result = self.tx.start_send_unpin(item);
        match result {
            Ok(()) => Ok(()),
            Err(_) => self.try_get_result(),
        }
    }
    fn poll_flush(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), Self::Error>> {
        let result = std::task::ready!(self.tx.poll_flush_unpin(cx));
        match result {
            Ok(()) => std::task::Poll::Ready(Ok(())),
            Err(_) => std::task::Poll::Ready(self.try_get_result()),
        }
    }
    fn poll_close(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), Self::Error>> {
        let result = std::task::ready!(self.tx.poll_close_unpin(cx));
        match result {
            Ok(()) => std::task::Poll::Ready(Ok(())),
            Err(_) => std::task::Poll::Ready(self.try_get_result()),
        }
    }
}
