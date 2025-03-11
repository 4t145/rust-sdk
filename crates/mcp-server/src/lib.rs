use std::{
    pin::Pin,
    task::{Context, Poll},
};

use futures::{Future, Sink, Stream};
use mcp_core::{
    Content, ResourceContents, ToolError,
    handler::{PromptError, ResourceError},
    prompt::{Prompt, PromptMessage, PromptMessageRole},
    protocol::{
        CallToolResult, GetPromptResult, Implementation, InitializeResult, JsonRpcError,
        JsonRpcMessage, JsonRpcRequest, JsonRpcResponse, JsonRpcVersion2_0, ListPromptsResult,
        ListResourcesResult, ListToolsResult, ReadResourceResult, ServerCapabilities,
    },
};
use pin_project::pin_project;
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};
use tower_service::Service;

mod errors;
pub use errors::{BoxError, RouterError, ServerError, TransportError};

pub mod router;
pub use router::Router;

/// A transport layer that handles JSON-RPC messages over byte
#[pin_project]
pub struct ByteTransport<R, W> {
    // Reader is a BufReader on the underlying stream (stdin or similar) buffering
    // the underlying data across poll calls, we clear one line (\n) during each
    // iteration of poll_next from this buffer
    #[pin]
    reader: BufReader<R>,
    #[pin]
    writer: W,
    send_item: Option<Vec<u8>>,
}

impl<R, W> ByteTransport<R, W>
where
    R: AsyncRead,
    W: AsyncWrite,
{
    pub fn new(reader: R, writer: W) -> Self {
        Self {
            // Default BufReader capacity is 8 * 1024, increase this to 2MB to the file size limit
            // allows the buffer to have the capacity to read very large calls
            reader: BufReader::with_capacity(2 * 1024 * 1024, reader),
            writer,
            send_item: None,
        }
    }
}

impl<R, W> Stream for ByteTransport<R, W>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    type Item = Result<JsonRpcMessage, TransportError>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let mut this = self.project();
        let mut buf = Vec::new();

        let mut reader = this.reader.as_mut();
        let mut read_future = Box::pin(reader.read_until(b'\n', &mut buf));
        match read_future.as_mut().poll(cx) {
            Poll::Ready(Ok(0)) => Poll::Ready(None), // EOF
            Poll::Ready(Ok(_)) => {
                // Convert to UTF-8 string
                let line = match String::from_utf8(buf) {
                    Ok(s) => s,
                    Err(e) => return Poll::Ready(Some(Err(TransportError::Utf8(e)))),
                };
                // Log incoming message here before serde conversion to
                // track incomplete chunks which are not valid JSON
                tracing::info!(json = %line, "incoming message");

                // Parse JSON and validate message format
                match serde_json::from_str::<serde_json::Value>(&line) {
                    Ok(value) => {
                        // Validate basic JSON-RPC structure
                        if !value.is_object() {
                            return Poll::Ready(Some(Err(TransportError::InvalidMessage(
                                "Message must be a JSON object".into(),
                            ))));
                        }
                        let obj = value.as_object().unwrap(); // Safe due to check above

                        // Check jsonrpc version field
                        if !obj.contains_key("jsonrpc") || obj["jsonrpc"] != "2.0" {
                            return Poll::Ready(Some(Err(TransportError::InvalidMessage(
                                "Missing or invalid jsonrpc version".into(),
                            ))));
                        }

                        // Now try to parse as proper message
                        match serde_json::from_value::<JsonRpcMessage>(value) {
                            Ok(msg) => Poll::Ready(Some(Ok(msg))),
                            Err(e) => Poll::Ready(Some(Err(TransportError::Json(e)))),
                        }
                    }
                    Err(e) => Poll::Ready(Some(Err(TransportError::Json(e)))),
                }
            }
            Poll::Ready(Err(e)) => Poll::Ready(Some(Err(TransportError::Io(e)))),
            Poll::Pending => Poll::Pending,
        }
    }
}

impl<R, W> ByteTransport<R, W>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    pub async fn write_message(&mut self, msg: JsonRpcMessage) -> Result<(), std::io::Error> {
        let json = serde_json::to_string(&msg)?;
        Pin::new(&mut self.writer)
            .write_all(json.as_bytes())
            .await?;
        Pin::new(&mut self.writer).write_all(b"\n").await?;
        Pin::new(&mut self.writer).flush().await?;
        Ok(())
    }
}

#[derive(Copy, Clone)]
pub struct McpServerContext<'a> {
    /// Sink to send messages to client
    json_rpc_tx: &'a tokio::sync::mpsc::Sender<JsonRpcMessage>,
}
pub trait McpServerHandler {
    fn name(&self) -> String;
    // in the protocol, instructions are optional but we make it required
    fn instructions(&self, context: McpServerContext) -> String;
    fn capabilities(&self, context: McpServerContext) -> ServerCapabilities;
    fn list_tools(
        &self,
        context: McpServerContext,
    ) -> impl Future<Output = Vec<mcp_core::tool::Tool>> + Send;
    fn call_tool(
        &self,
        context: McpServerContext,
        tool_name: &str,
        arguments: Value,
    ) -> impl Future<Output = Result<Vec<Content>, ToolError>> + Send;
    fn list_resources(
        &self,
        context: McpServerContext,
    ) -> impl Future<Output = Vec<mcp_core::resource::Resource>> + Send;
    fn read_resource(
        &self,
        context: McpServerContext,
        uri: &str,
    ) -> impl Future<Output = Result<String, ResourceError>> + Send;
    fn list_prompts(&self, context: McpServerContext) -> impl Future<Output = Vec<Prompt>> + Send;
    fn get_prompt(
        &self,
        context: McpServerContext,
        prompt_name: &str,
    ) -> impl Future<Output = Result<String, PromptError>> + Send;
}

pub struct McpServer<H> {
    handler: H,
}

impl<H> McpServer<H>
where
    H: McpServerHandler,
{
    async fn handle_initialize(
        &self,
        req: JsonRpcRequest,
        context: McpServerContext<'_>,
    ) -> Result<JsonRpcResponse, RouterError> {
        let result = InitializeResult {
            protocol_version: "2024-11-05".to_string(),
            capabilities: self.handler.capabilities(context).clone(),
            server_info: Implementation {
                name: self.handler.name(),
                version: env!("CARGO_PKG_VERSION").to_string(),
            },
            instructions: Some(self.handler.instructions(context)),
        };

        let mut response = self.create_response(req.id);
        response.result = Some(
            serde_json::to_value(result)
                .map_err(|e| RouterError::Internal(format!("JSON serialization error: {}", e)))?,
        );

        Ok(response)
    }

    async fn handle_tools_list(
        &self,
        req: JsonRpcRequest,
        context: McpServerContext<'_>,
    ) -> Result<JsonRpcResponse, RouterError> {
        let tools = self.handler.list_tools(context).await;

        let result = ListToolsResult {
            tools,
            next_cursor: None,
        };
        let mut response = self.create_response(req.id);
        response.result = Some(
            serde_json::to_value(result)
                .map_err(|e| RouterError::Internal(format!("JSON serialization error: {}", e)))?,
        );

        Ok(response)
    }

    async fn handle_tools_call(
        &self,
        req: JsonRpcRequest,
        context: McpServerContext<'_>,
    ) -> Result<JsonRpcResponse, RouterError> {
        let params = req
            .params
            .ok_or_else(|| RouterError::InvalidParams("Missing parameters".into()))?;

        let name = params
            .get("name")
            .and_then(Value::as_str)
            .ok_or_else(|| RouterError::InvalidParams("Missing tool name".into()))?;

        let arguments = params.get("arguments").cloned().unwrap_or(Value::Null);

        let result = match self.handler.call_tool(context, name, arguments).await {
            Ok(result) => CallToolResult {
                content: result,
                is_error: None,
            },
            Err(err) => CallToolResult {
                content: vec![Content::text(err.to_string())],
                is_error: Some(true),
            },
        };

        let mut response = self.create_response(req.id);
        response.result = Some(
            serde_json::to_value(result)
                .map_err(|e| RouterError::Internal(format!("JSON serialization error: {}", e)))?,
        );

        Ok(response)
    }

    async fn handle_resources_list(
        &self,
        req: JsonRpcRequest,
        context: McpServerContext<'_>,
    ) -> Result<JsonRpcResponse, RouterError> {
        let resources = self.handler.list_resources(context).await;

        let result = ListResourcesResult {
            resources,
            next_cursor: None,
        };
        let mut response = self.create_response(req.id);
        response.result = Some(
            serde_json::to_value(result)
                .map_err(|e| RouterError::Internal(format!("JSON serialization error: {}", e)))?,
        );

        Ok(response)
    }

    async fn handle_resources_read(
        &self,
        req: JsonRpcRequest,
        context: McpServerContext<'_>,
    ) -> Result<JsonRpcResponse, RouterError> {
        let params = req
            .params
            .ok_or_else(|| RouterError::InvalidParams("Missing parameters".into()))?;

        let uri = params
            .get("uri")
            .and_then(Value::as_str)
            .ok_or_else(|| RouterError::InvalidParams("Missing resource URI".into()))?;

        let contents = self
            .handler
            .read_resource(context, uri)
            .await
            .map_err(RouterError::from)?;

        let result = ReadResourceResult {
            contents: vec![ResourceContents::TextResourceContents {
                uri: uri.to_string(),
                mime_type: Some("text/plain".to_string()),
                text: contents,
            }],
        };

        let mut response = self.create_response(req.id);
        response.result = Some(
            serde_json::to_value(result)
                .map_err(|e| RouterError::Internal(format!("JSON serialization error: {}", e)))?,
        );

        Ok(response)
    }

    async fn handle_prompts_list(
        &self,
        req: JsonRpcRequest,
        context: McpServerContext<'_>,
    ) -> Result<JsonRpcResponse, RouterError> {
        let prompts = self.handler.list_prompts(context).await;

        let result = ListPromptsResult { prompts };

        let mut response = self.create_response(req.id);
        response.result = Some(
            serde_json::to_value(result)
                .map_err(|e| RouterError::Internal(format!("JSON serialization error: {}", e)))?,
        );

        Ok(response)
    }

    async fn handle_prompts_get(
        &self,
        req: JsonRpcRequest,
        context: McpServerContext<'_>,
    ) -> Result<JsonRpcResponse, RouterError> {
        // Validate and extract parameters
        let params = req
            .params
            .ok_or_else(|| RouterError::InvalidParams("Missing parameters".into()))?;

        // Extract "name" field
        let prompt_name = params
            .get("name")
            .and_then(Value::as_str)
            .ok_or_else(|| RouterError::InvalidParams("Missing prompt name".into()))?;

        // Extract "arguments" field
        let arguments = params
            .get("arguments")
            .and_then(Value::as_object)
            .ok_or_else(|| RouterError::InvalidParams("Missing arguments object".into()))?;

        // Fetch the prompt definition first
        let prompt = self
            .handler
            .list_prompts(context)
            .await
            .into_iter()
            .find(|p| p.name == prompt_name)
            .ok_or_else(|| {
                RouterError::PromptNotFound(format!("Prompt '{}' not found", prompt_name))
            })?;

        // Validate required arguments
        if let Some(args) = &prompt.arguments {
            for arg in args {
                if arg.required.is_some()
                    && arg.required.unwrap()
                    && (!arguments.contains_key(&arg.name)
                        || arguments
                            .get(&arg.name)
                            .and_then(Value::as_str)
                            .is_none_or(str::is_empty))
                {
                    return Err(RouterError::InvalidParams(format!(
                        "Missing required argument: '{}'",
                        arg.name
                    )));
                }
            }
        }

        // Now get the prompt content
        let description = self
            .handler
            .get_prompt(context, prompt_name)
            .await
            .map_err(|e| RouterError::Internal(e.to_string()))?;

        // Validate prompt arguments for potential security issues from user text input
        // Checks:
        // - Prompt must be less than 10000 total characters
        // - Argument keys must be less than 1000 characters
        // - Argument values must be less than 1000 characters
        // - Dangerous patterns, eg "../", "//", "\\\\", "<script>", "{{", "}}"
        for (key, value) in arguments.iter() {
            // Check for empty or overly long keys/values
            if key.is_empty() || key.len() > 1000 {
                return Err(RouterError::InvalidParams(
                    "Argument keys must be between 1-1000 characters".into(),
                ));
            }

            let value_str = value.as_str().unwrap_or_default();
            if value_str.len() > 1000 {
                return Err(RouterError::InvalidParams(
                    "Argument values must not exceed 1000 characters".into(),
                ));
            }

            // Check for potentially dangerous patterns
            let dangerous_patterns = ["../", "//", "\\\\", "<script>", "{{", "}}"];
            for pattern in dangerous_patterns {
                if key.contains(pattern) || value_str.contains(pattern) {
                    return Err(RouterError::InvalidParams(format!(
                        "Arguments contain potentially unsafe pattern: {}",
                        pattern
                    )));
                }
            }
        }

        // Validate the prompt description length
        if description.len() > 10000 {
            return Err(RouterError::Internal(
                "Prompt description exceeds maximum allowed length".into(),
            ));
        }

        // Create a mutable copy of the description to fill in arguments
        let mut description_filled = description.clone();

        // Replace each argument placeholder with its value from the arguments object
        for (key, value) in arguments {
            let placeholder = format!("{{{}}}", key);
            description_filled =
                description_filled.replace(&placeholder, value.as_str().unwrap_or_default());
        }

        let messages = vec![PromptMessage::new_text(
            PromptMessageRole::User,
            description_filled.to_string(),
        )];

        // Build the final response
        let mut response = self.create_response(req.id);
        response.result = Some(
            serde_json::to_value(GetPromptResult {
                description: Some(description_filled),
                messages,
            })
            .map_err(|e| RouterError::Internal(format!("JSON serialization error: {}", e)))?,
        );
        Ok(response)
    }
    fn create_response(&self, id: Option<u64>) -> JsonRpcResponse {
        JsonRpcResponse {
            jsonrpc: Default::default(),
            id,
            result: None,
            error: None,
        }
    }
    pub async fn handle(
        &self,
        context: McpServerContext<'_>,
        request: JsonRpcRequest,
    ) -> Result<JsonRpcResponse, BoxError> {
        let result = match request.method.as_str() {
            "initialize" => self.handle_initialize(request, context).await,
            "tools/list" => self.handle_tools_list(request, context).await,
            "tools/call" => self.handle_tools_call(request, context).await,
            "resources/list" => self.handle_resources_list(request, context).await,
            "resources/read" => self.handle_resources_read(request, context).await,
            "prompts/list" => self.handle_prompts_list(request, context).await,
            "prompts/get" => self.handle_prompts_get(request, context).await,
            _ => {
                let mut response = self.create_response(request.id);
                response.error = Some(RouterError::MethodNotFound(request.method).into());
                Ok(response)
            }
        };

        result.map_err(BoxError::from)
    }
    pub async fn run<T>(self, mut transport: T) -> Result<(), ServerError>
    where
        T: McpServerTransport,
    {
        use futures::{SinkExt, StreamExt};

        tracing::info!("Server started");
        let (mut sink, mut stream) = transport.split();
        let message_sink = 
        // let mut stream = std::pin::pin!(stream);
        while let Some(msg_result) = stream.next().await {
            let _span = tracing::span!(tracing::Level::INFO, "message_processing");
            let _enter = _span.enter();
            match msg_result {
                Ok(msg) => {
                    match msg {
                        JsonRpcMessage::Request(request) => {
                            // Serialize request for logging
                            let id = request.id;
                            let request_json = serde_json::to_string(&request)
                                .unwrap_or_else(|_| "Failed to serialize request".to_string());

                            tracing::info!(
                                request_id = ?id,
                                method = ?request.method,
                                json = %request_json,
                                "Received request"
                            );

                            // Process the request using our service
                            let response = match self.handle(request).await {
                                Ok(resp) => resp,
                                Err(e) => {
                                    let error_msg = e.into().to_string();
                                    tracing::error!(error = %error_msg, "Request processing failed");
                                    JsonRpcResponse {
                                        jsonrpc: JsonRpcVersion2_0,
                                        id,
                                        result: None,
                                        error: Some(mcp_core::protocol::ErrorData {
                                            code: mcp_core::protocol::INTERNAL_ERROR,
                                            message: error_msg,
                                            data: None,
                                        }),
                                    }
                                }
                            };

                            // Serialize response for logging
                            let response_json = serde_json::to_string(&response)
                                .unwrap_or_else(|_| "Failed to serialize response".to_string());

                            tracing::info!(
                                response_id = ?response.id,
                                json = %response_json,
                                "Sending response"
                            );
                            // Send the response back
                            if let Err(e) = sink.send(JsonRpcMessage::Response(response)).await {
                                return Err(ServerError::Transport(TransportError::Io(e)));
                            }
                        }
                        JsonRpcMessage::Response(_)
                        | JsonRpcMessage::Notification(_)
                        | JsonRpcMessage::Nil
                        | JsonRpcMessage::Error(_) => {
                            // Ignore responses, notifications and nil messages for now
                            continue;
                        }
                    }
                }
                Err(e) => {
                    // Convert transport error to JSON-RPC error response
                    let error = match e {
                        TransportError::Json(_) | TransportError::InvalidMessage(_) => {
                            mcp_core::protocol::ErrorData {
                                code: mcp_core::protocol::PARSE_ERROR,
                                message: e.to_string(),
                                data: None,
                            }
                        }
                        TransportError::Protocol(_) => mcp_core::protocol::ErrorData {
                            code: mcp_core::protocol::INVALID_REQUEST,
                            message: e.to_string(),
                            data: None,
                        },
                        _ => mcp_core::protocol::ErrorData {
                            code: mcp_core::protocol::INTERNAL_ERROR,
                            message: e.to_string(),
                            data: None,
                        },
                    };

                    let error_response = JsonRpcMessage::Error(JsonRpcError {
                        jsonrpc: JsonRpcVersion2_0,
                        id: None,
                        error,
                    });

                    if let Err(e) = sink.send(error_response).await {
                        return Err(ServerError::Transport(TransportError::Io(e)));
                    }
                }
            }
        }

        Ok(())
    }
}

pub trait McpServerTransport:
    Stream<Item = Result<JsonRpcMessage, TransportError>> + Sink<JsonRpcMessage, Error = std::io::Error>
{
}

/// The main server type that processes incoming requests
pub struct Server<S> {
    service: S,
}

impl<S> Server<S>
where
    S: Service<JsonRpcRequest, Response = JsonRpcResponse> + Send,
    S::Error: Into<BoxError>,
    S::Future: Send,
{
    pub fn new(service: S) -> Self {
        Self { service }
    }

    // TODO transport trait instead of byte transport if we implement others
    pub async fn run<T>(self, mut transport: T) -> Result<(), ServerError>
    where
        T: McpServerTransport,
    {
        use futures::{SinkExt, StreamExt};
        let mut service = self.service;

        tracing::info!("Server started");
        let (mut sink, mut stream) = transport.split();
        // let mut stream = std::pin::pin!(stream);
        while let Some(msg_result) = stream.next().await {
            let _span = tracing::span!(tracing::Level::INFO, "message_processing");
            let _enter = _span.enter();
            match msg_result {
                Ok(msg) => {
                    match msg {
                        JsonRpcMessage::Request(request) => {
                            // Serialize request for logging
                            let id = request.id;
                            let request_json = serde_json::to_string(&request)
                                .unwrap_or_else(|_| "Failed to serialize request".to_string());

                            tracing::info!(
                                request_id = ?id,
                                method = ?request.method,
                                json = %request_json,
                                "Received request"
                            );

                            // Process the request using our service
                            let response = match service.call(request).await {
                                Ok(resp) => resp,
                                Err(e) => {
                                    let error_msg = e.into().to_string();
                                    tracing::error!(error = %error_msg, "Request processing failed");
                                    JsonRpcResponse {
                                        jsonrpc: JsonRpcVersion2_0,
                                        id,
                                        result: None,
                                        error: Some(mcp_core::protocol::ErrorData {
                                            code: mcp_core::protocol::INTERNAL_ERROR,
                                            message: error_msg,
                                            data: None,
                                        }),
                                    }
                                }
                            };

                            // Serialize response for logging
                            let response_json = serde_json::to_string(&response)
                                .unwrap_or_else(|_| "Failed to serialize response".to_string());

                            tracing::info!(
                                response_id = ?response.id,
                                json = %response_json,
                                "Sending response"
                            );
                            // Send the response back
                            if let Err(e) = sink.send(JsonRpcMessage::Response(response)).await {
                                return Err(ServerError::Transport(TransportError::Io(e)));
                            }
                        }
                        JsonRpcMessage::Response(_)
                        | JsonRpcMessage::Notification(_)
                        | JsonRpcMessage::Nil
                        | JsonRpcMessage::Error(_) => {
                            // Ignore responses, notifications and nil messages for now
                            continue;
                        }
                    }
                }
                Err(e) => {
                    // Convert transport error to JSON-RPC error response
                    let error = match e {
                        TransportError::Json(_) | TransportError::InvalidMessage(_) => {
                            mcp_core::protocol::ErrorData {
                                code: mcp_core::protocol::PARSE_ERROR,
                                message: e.to_string(),
                                data: None,
                            }
                        }
                        TransportError::Protocol(_) => mcp_core::protocol::ErrorData {
                            code: mcp_core::protocol::INVALID_REQUEST,
                            message: e.to_string(),
                            data: None,
                        },
                        _ => mcp_core::protocol::ErrorData {
                            code: mcp_core::protocol::INTERNAL_ERROR,
                            message: e.to_string(),
                            data: None,
                        },
                    };

                    let error_response = JsonRpcMessage::Error(JsonRpcError {
                        jsonrpc: JsonRpcVersion2_0,
                        id: None,
                        error,
                    });

                    if let Err(e) = sink.send(error_response).await {
                        return Err(ServerError::Transport(TransportError::Io(e)));
                    }
                }
            }
        }

        Ok(())
    }
}

// Define a specific service implementation that we need for any
// Any router implements this
pub trait BoundedService:
    Service<
        JsonRpcRequest,
        Response = JsonRpcResponse,
        Error = BoxError,
        Future = Pin<Box<dyn Future<Output = Result<JsonRpcResponse, BoxError>> + Send>>,
    > + Send
    + 'static
{
}

// Implement it for any type that meets the bounds
impl<T> BoundedService for T where
    T: Service<
            JsonRpcRequest,
            Response = JsonRpcResponse,
            Error = BoxError,
            Future = Pin<Box<dyn Future<Output = Result<JsonRpcResponse, BoxError>> + Send>>,
        > + Send
        + 'static
{
}
