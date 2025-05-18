use std::{convert::Infallible, fmt::Display, time::Duration};

use crate::transport::common::http_header::{HEADER_LAST_EVENT_ID, HEADER_SESSION_ID};

use super::session::{EventId, SessionHandle, SessionId, SessionWorker, StreamableHttpMessageReceiver};
use bytes::Bytes;
use futures::{future::BoxFuture, Stream};
use http::{Method, Response, StatusCode};
use http_body_util::{BodyExt, combinators::UnsyncBoxBody};
use tokio_stream::wrappers::ReceiverStream;
use tower_service::Service;
pub trait SessionManager: Clone {
    type Error: std::error::Error;
    fn get_session(
        &self,
        id: SessionId,
    ) -> impl Future<Output = Result<SessionHandle, Self::Error>>;
}
pub type BoxedError = Box<dyn std::error::Error + Send + Sync>;
pub type BoxedBody = UnsyncBoxBody<Bytes, BoxedError>;
pub struct StreamableHttpService<M> {
    context: StreamableHttpContext<M>,
}
#[derive(Debug, Clone)]
pub(crate) struct StreamableHttpContext<M> {
    session_manager: M,
    transport_tx: tokio::sync::mpsc::UnboundedSender<SessionWorker>,
    sse_ping_interval: Duration,
}
impl<ReqBody, M> Service<http::Request<ReqBody>> for StreamableHttpContext<M>
where
    ReqBody: http_body::Body,
    M: SessionManager,
{
    type Error = Infallible;

    type Response = Response<BoxedBody>;

    type Future = BoxFuture<'static, Result<Self::Response, Self::Error>>;

    fn poll_ready(
        &mut self,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), Self::Error>> {
        todo!()
    }

    fn call(&mut self, req: http::Request<ReqBody>) -> Self::Future {
        // route by method
        let method = req.method().clone();
        match method {
            Method::GET => {
                todo!()
            }
            Method::POST => {
                todo!()
            }
            Method::DELETE => {
                todo!()
            }
            _ => {
                todo!()
            }
        }
    }
}

fn never<T>(_: Infallible) -> T {
    unreachable!()
}

fn code_message_response(code: StatusCode, message: impl Display) -> Response<BoxedBody> {
    Response::builder()
        .status(code)
        .header(http::header::CONTENT_TYPE, "text/plain")
        .body(UnsyncBoxBody::new(
            http_body_util::Full::new(message.to_string().into()).map_err(never),
        ))
        .expect("fail to build http response body")
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
impl<M: SessionManager> StreamableHttpContext<M> {
    async fn handle_get<ReqBody>(&self, request: http::Request<ReqBody>) -> Result<Response<BoxedBody>, Response<BoxedBody>>{
        let header_map = request.headers().clone();
        let session_id = header_map
            .get(HEADER_SESSION_ID)
            .and_then(|v| v.to_str().ok());
        if let Some(session_id) = session_id {
            let last_event_id = header_map
                .get(HEADER_LAST_EVENT_ID)
                .and_then(|v| v.to_str().ok());
            let session_id: SessionId = session_id.to_owned().into();
            let session = {
                self.session_manager.get_session(session_id.clone()).await.map_err(|e| {
                    code_message_response(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        format!("session manager error {e}"),
                    )
                })?
            };
            match last_event_id {
                Some(last_event_id) => {
                    let last_event_id = last_event_id.parse::<EventId>().map_err(|e| {
                        code_message_response(StatusCode::BAD_REQUEST, format!("invalid event_id {e}"))
                    })?;
                    let receiver = session.resume(last_event_id).await.map_err(|e| {
                        code_message_response(
                            StatusCode::INTERNAL_SERVER_ERROR,
                            format!("resume error {e}"),
                        )
                    })?;
                    let stream = receiver_as_stream(receiver);
                    Ok(Sse::new(stream)
                        .keep_alive(KeepAlive::new().interval(app.sse_ping_interval)))
                }
                None => {
                    let receiver = session.establish_common_channel().await.map_err(|e| {
                        (
                            StatusCode::INTERNAL_SERVER_ERROR,
                            format!("establish common channel error {e}"),
                        )
                            .into_response()
                    })?;
                    let stream = receiver_as_stream(receiver);
                    Ok(Sse::new(stream)
                        .keep_alive(KeepAlive::new().interval(app.sse_ping_interval)))
                }
            }
        } else {
            Err((StatusCode::BAD_REQUEST, "missing session id").into_response())
        }
    }
}
