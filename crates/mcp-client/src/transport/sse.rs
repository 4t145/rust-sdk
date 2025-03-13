use eventsource_client::{
    BoxStream, Client as EventSourceClient, ClientBuilder, Error as SseError, SSE,
};
use futures::{FutureExt, Sink, Stream, StreamExt};
use mcp_core::schema::{ClientJsonRpcMessage, ServerJsonRpcMessage};
use reqwest::{Client as HttpClient, header::HeaderMap};
use std::{collections::VecDeque, sync::Arc};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum SseTransportError {
    #[error("SSE error: {0}")]
    Sse(#[from] SseError),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Reqwest error: {0}")]
    Reqwest(#[from] reqwest::Error),
    #[error("unexpected end of stream")]
    UnexpectedEndOfStream,
}
pub struct SseTransport {
    http_client: HttpClient,
    event_source: BoxStream<Result<SSE, SseError>>,
    post_url: Arc<str>,
    _sse_url: Arc<str>,
    #[allow(clippy::type_complexity)]
    request_queue: VecDeque<tokio::sync::oneshot::Receiver<Result<(), SseTransportError>>>,
}

impl SseTransport {
    pub async fn start(url: &str, headers: HeaderMap) -> Result<Self, SseTransportError> {
        let mut sse_client_builder = ClientBuilder::for_url(url)?;
        for (name, value) in &headers {
            if let Ok(value) = std::str::from_utf8(value.as_bytes()) {
                sse_client_builder = sse_client_builder.header(name.as_str(), value)?;
            }
        }
        let client = sse_client_builder.build();
        let mut event_stream = client.stream();
        let first_event = loop {
            let next_event = event_stream
                .next()
                .await
                .ok_or(SseTransportError::UnexpectedEndOfStream)??;
            match next_event {
                SSE::Event(event) => {
                    break event;
                }
                SSE::Comment(_) => continue,
            }
        };
        let post_uri = format!("{}{}", url, first_event.data);
        let sse_uri = url.to_string();
        Ok(SseTransport {
            http_client: HttpClient::builder().default_headers(headers).build()?,
            event_source: event_stream,
            post_url: Arc::from(post_uri),
            _sse_url: Arc::from(sse_uri),
            request_queue: Default::default(),
        })
    }
}

impl Stream for SseTransport {
    type Item = ServerJsonRpcMessage;

    fn poll_next(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        let event = std::task::ready!(self.event_source.poll_next_unpin(cx));
        match event {
            Some(Ok(SSE::Event(event))) => match serde_json::from_str(&event.data) {
                Ok(message) => std::task::Poll::Ready(Some(message)),
                Err(e) => {
                    tracing::error!(error = %e, "failed to parse json rpc request");
                    self.poll_next(cx)
                }
            },
            Some(Ok(SSE::Comment(_))) => self.poll_next(cx),
            Some(Err(e)) => {
                tracing::error!(error = %e, "sse event stream encounter an error");
                std::task::Poll::Ready(None)
            }
            None => std::task::Poll::Ready(None),
        }
    }
}

impl Sink<ClientJsonRpcMessage> for SseTransport {
    type Error = SseTransportError;

    fn poll_ready(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), Self::Error>> {
        const QUEUE_SIZE: usize = 16;
        if self.request_queue.len() >= QUEUE_SIZE {
            std::task::ready!(
                self.request_queue
                    .front_mut()
                    .expect("queue is not empty")
                    .poll_unpin(cx)
            )
            .expect("sender shall not drop")?;
        }
        std::task::Poll::Ready(Ok(()))
    }

    fn start_send(
        mut self: std::pin::Pin<&mut Self>,
        item: ClientJsonRpcMessage,
    ) -> Result<(), Self::Error> {
        let client = self.http_client.clone();
        let uri = self.post_url.clone();
        let (tx, rx) = tokio::sync::oneshot::channel();
        tokio::spawn(async move {
            let result = client
                .post(uri.as_ref())
                .json(&item)
                .send()
                .await
                .and_then(|resp| resp.error_for_status())
                .map_err(SseTransportError::from)
                .map(drop);
            let _ = tx.send(result);
        });
        self.as_mut().request_queue.push_back(rx);
        Ok(())
    }

    fn poll_flush(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), Self::Error>> {
        let queue = &mut self.as_mut().request_queue;
        while let Some(fut) = queue.front_mut() {
            std::task::ready!(fut.poll_unpin(cx)).expect("sender shall not drop")?;
            queue.pop_front();
        }
        std::task::Poll::Ready(Ok(()))
    }

    fn poll_close(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), Self::Error>> {
        self.poll_flush(cx)
    }
}
