//！ reference: https://html.spec.whatwg.org/multipage/server-sent-events.html
use std::{collections::VecDeque, sync::Arc, time::Duration};

use futures::{FutureExt, Sink, Stream, StreamExt, future::BoxFuture, stream::BoxStream};
use reqwest::{
    Client as HttpClient, IntoUrl, Url,
    header::{ACCEPT, HeaderValue},
};
use sse_stream::{Error as SseError, Sse, SseStream};
use thiserror::Error;

use crate::model::{ClientJsonRpcMessage, ServerJsonRpcMessage};
const EVENT_STREAM_MIME_TYPE: &str = "text/event-stream";
const JSON_MIME_TYPE: &str = "application/json";
const HEADER_LAST_EVENT_ID: &str = "Last-Event-ID";

#[derive(Error, Debug)]
pub enum StreamableHttpError<E: std::error::Error + Send + Sync + 'static> {
    #[error("SSE error: {0}")]
    Sse(#[from] SseError),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Transport error: {0}")]
    Transport(E),
    #[error("unexpected end of stream")]
    UnexpectedEndOfStream,
    #[error("Url error: {0}")]
    Url(#[from] url::ParseError),
    #[error("Unexpected content type: {0:?}")]
    UnexpectedContentType(Option<HeaderValue>),
}

type SseStreamFuture<E> =
    BoxFuture<'static, Result<BoxStream<'static, Result<Sse, SseError>>, StreamableHttpError<E>>>;

enum SseTransportState<E: std::error::Error + Send + Sync + 'static> {
    Connected(BoxStream<'static, Result<Sse, SseError>>),
    Retrying {
        times: usize,
        fut: SseStreamFuture<E>,
    },
    Fatal {
        reason: String,
    },
}

#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq)]
pub struct SseTransportRetryConfig {
    pub max_times: Option<usize>,
    pub min_duration: Duration,
}
impl SseTransportRetryConfig {
    pub const DEFAULT_MIN_DURATION: Duration = Duration::from_millis(1000);
}
impl Default for SseTransportRetryConfig {
    fn default() -> Self {
        Self {
            max_times: None,
            min_duration: Self::DEFAULT_MIN_DURATION,
        }
    }
}

impl From<reqwest::Error> for StreamableHttpError<reqwest::Error> {
    fn from(e: reqwest::Error) -> Self {
        StreamableHttpError::Transport(e)
    }
}

pub enum StreamableHttpPostResponse {
    Accepted,
    Json(ServerJsonRpcMessage),
    Sse(BoxStream<'static, Result<Sse, SseError>>),
}

pub trait StreamableHttpClient {
    type Error: std::error::Error + Send + Sync + 'static;
    fn post_message(
        &self,
        message: ClientJsonRpcMessage,
    ) -> impl Future<Output = Result<StreamableHttpPostResponse, StreamableHttpError<Self::Error>>>
    + Send
    + '_;
    fn delete_session(
        &self,
        session: String,
    ) -> impl Future<Output = Result<(), StreamableHttpError<Self::Error>>> + Send + '_;
    fn get_stream(
        &self,
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

#[derive(Clone)]
pub struct ReqwestSseClient {
    http_client: HttpClient,
    url: Url,
}

impl ReqwestSseClient {
    pub fn new<U>(url: U) -> Result<Self, StreamableHttpError<reqwest::Error>>
    where
        U: IntoUrl,
    {
        let url = url.into_url()?;
        Ok(Self {
            http_client: HttpClient::default(),
            url,
        })
    }

    pub async fn new_with_timeout<U>(
        url: U,
        timeout: Duration,
    ) -> Result<Self, StreamableHttpError<reqwest::Error>>
    where
        U: IntoUrl,
    {
        let mut client = HttpClient::builder();
        client = client.timeout(timeout);
        let client = client.build()?;
        let url = url.into_url()?;
        Ok(Self {
            http_client: client,
            url,
        })
    }

    pub async fn new_with_client<U>(
        url: U,
        client: HttpClient,
    ) -> Result<Self, StreamableHttpError<reqwest::Error>>
    where
        U: IntoUrl,
    {
        let url = url.into_url()?;
        Ok(Self {
            http_client: client,
            url,
        })
    }
}

impl StreamableHttpClient for ReqwestSseClient {
    type Error = reqwest::Error;
    async fn get_stream(
        &self,
        last_event_id: Option<String>,
    ) -> Result<BoxStream<'static, Result<Sse, SseError>>, StreamableHttpError<Self::Error>> {
        let mut request_builder = self
            .http_client
            .get(self.url.clone())
            .header(ACCEPT, EVENT_STREAM_MIME_TYPE);
        if let Some(last_event_id) = last_event_id {
            request_builder = request_builder.header(HEADER_LAST_EVENT_ID, last_event_id);
        }
        let response = request_builder.send().await?;
        let response = response.error_for_status()?;
        match response.headers().get(reqwest::header::CONTENT_TYPE) {
            Some(ct) => {
                if !ct.as_bytes().starts_with(EVENT_STREAM_MIME_TYPE.as_bytes()) {
                    return Err(StreamableHttpError::UnexpectedContentType(Some(ct.clone())));
                }
            }
            None => {
                return Err(StreamableHttpError::UnexpectedContentType(None));
            }
        }
        let event_stream = SseStream::from_byte_stream(response.bytes_stream()).boxed();
        Ok(event_stream)
    }

    async fn delete_session(
        &self,
        session: String,
    ) -> Result<(), StreamableHttpError<Self::Error>> {
        let response = self
            .http_client
            .delete(self.url.join(&session)?)
            .send()
            .await?;
        // if method no allowed
        if response.status() == reqwest::StatusCode::METHOD_NOT_ALLOWED {
            tracing::debug!("this server doesn't support deleting session");
            return Ok(());
        }
        let _response = response.error_for_status()?;
        Ok(())
    }

    async fn post_message(
        &self,
        message: ClientJsonRpcMessage,
    ) -> Result<StreamableHttpPostResponse, StreamableHttpError<Self::Error>> {
        let response = self
            .http_client
            .post(self.url.clone())
            .header(ACCEPT, EVENT_STREAM_MIME_TYPE)
            .header(ACCEPT, JSON_MIME_TYPE)
            .json(&message)
            .send()
            .await?;
        if response.status() == reqwest::StatusCode::ACCEPTED {
            return Ok(StreamableHttpPostResponse::Accepted);
        }
        let content_type = response.headers().get(reqwest::header::CONTENT_TYPE);
        match content_type {
            Some(ct) if ct.as_bytes().starts_with(EVENT_STREAM_MIME_TYPE.as_bytes()) => {
                let event_stream = SseStream::from_byte_stream(response.bytes_stream()).boxed();
                Ok(StreamableHttpPostResponse::Sse(event_stream))
            }
            Some(ct) if ct.as_bytes().starts_with(JSON_MIME_TYPE.as_bytes()) => {
                let message: ServerJsonRpcMessage = response.json().await?;
                Ok(StreamableHttpPostResponse::Json(message))
            }
            _ => {
                // unexpected content type
                tracing::error!("unexpected content type: {:?}", content_type);
                Err(StreamableHttpError::UnexpectedContentType(
                    content_type.cloned(),
                ))
            }
        }
    }
}

/// # Transport for client sse
///
/// Call [`SseTransport::start`] to create a  new transport from url.
///
/// Call [`SseTransport::start_with_client`] to create a new transport with a customized reqwest client.
pub struct SseTransport<C: StreamableHttpClient> {
    client: Arc<C>,
    last_event_id: Option<String>,
    recommended_retry_duration_ms: Option<u64>,
    session_id: String,
    #[allow(clippy::type_complexity)]
    streams: BoxStream<'static, Result<Sse, SseError>>,
    pub retry_config: SseTransportRetryConfig,
}