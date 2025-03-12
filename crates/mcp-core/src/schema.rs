use std::{collections::BTreeMap, fmt::Pointer};

/// The protocol messages exchanged between client and server
use crate::{
    Role,
    content::Content,
    prompt::{Prompt, PromptMessage},
    resource::{Resource, ResourceContents},
    tool::Tool,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
type JsonObject<F = Value> = serde_json::Map<String, F>;
pub trait ConstString {
    const VALUE: &str;
}

macro_rules! const_string {
    ($name:ident = $value:literal) => {
        #[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
        pub struct $name;

        impl ConstString for $name {
            const VALUE: &str = $value;
        }

        impl Serialize for $name {
            fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
            where
                S: serde::Serializer,
            {
                $value.serialize(serializer)
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D>(deserializer: D) -> Result<$name, D::Error>
            where
                D: serde::Deserializer<'de>,
            {
                let s: String = Deserialize::deserialize(deserializer)?;
                if s == $value {
                    Ok($name)
                } else {
                    Err(serde::de::Error::custom(format!(concat!(
                        "expect const string value \"",
                        $value,
                        "\""
                    ))))
                }
            }
        }
    };
}

const_string!(JsonRpcVersion2_0 = "2.0");
const_string!(LatestProtocolVersion = "2024-11-05");

#[derive(Debug, Clone, Eq, PartialEq, Hash)]
pub enum NumberOrString {
    Number(u32),
    String(String),
}

impl std::fmt::Display for NumberOrString {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            NumberOrString::Number(n) => n.fmt(f),
            NumberOrString::String(s) => s.fmt(f),
        }
    }
}

impl Serialize for NumberOrString {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        match self {
            NumberOrString::Number(n) => n.serialize(serializer),
            NumberOrString::String(s) => s.serialize(serializer),
        }
    }
}

impl<'de> Deserialize<'de> for NumberOrString {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value: Value = Deserialize::deserialize(deserializer)?;
        match value {
            Value::Number(n) => Ok(NumberOrString::Number(
                n.as_u64()
                    .ok_or(serde::de::Error::custom("Expect an integer"))? as u32,
            )),
            Value::String(s) => Ok(NumberOrString::String(s)),
            _ => Err(serde::de::Error::custom("Expect number or string")),
        }
    }
}

pub type RequestId = NumberOrString;
pub type ProgressToken = NumberOrString;
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct WithMeta<P = JsonObject, M = ()> {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub _meta: Option<M>,
    #[serde(flatten)]
    pub inner: P,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RequestMeta {
    progress_token: ProgressToken,
}
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct Request<M = String, P = Option<WithMeta<JsonObject, RequestMeta>>> {
    pub method: M,
    // #[serde(skip_serializing_if = "Option::is_none")]
    pub params: P,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct Notification<M = String, P = Option<WithMeta<JsonObject, JsonObject>>> {
    pub method: M,
    pub params: P,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct JsonRpcRequest<R = Request> {
    pub jsonrpc: JsonRpcVersion2_0,
    pub id: RequestId,
    #[serde(flatten)]
    pub request: R,
}
type DefaultResponse = WithMeta<JsonObject, JsonObject>;
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct JsonRpcResponse<R = DefaultResponse> {
    pub jsonrpc: JsonRpcVersion2_0,
    pub id: RequestId,
    pub result: R,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct JsonRpcError {
    pub jsonrpc: JsonRpcVersion2_0,
    pub id: RequestId,
    pub error: ErrorData,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct JsonRpcNotification<N = Notification> {
    pub jsonrpc: JsonRpcVersion2_0,
    #[serde(flatten)]
    pub notification: N,
}

// Standard JSON-RPC error codes
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(transparent)]
pub struct ErrorCode(pub i32);

impl ErrorCode {
    pub const PARSE_ERROR: Self = Self(-32700);
    pub const INVALID_REQUEST: Self = Self(-32600);
    pub const METHOD_NOT_FOUND: Self = Self(-32601);
    pub const INVALID_PARAMS: Self = Self(-32602);
    pub const INTERNAL_ERROR: Self = Self(-32603);
}

/// Error information for JSON-RPC error responses.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct ErrorData {
    /// The error type that occurred.
    pub code: ErrorCode,

    /// A short description of the error. The message SHOULD be limited to a concise single sentence.
    pub message: String,

    /// Additional information about the error. The value of this member is defined by the
    /// sender (e.g. detailed error information, nested errors etc.).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

impl ErrorData {
    pub fn new(code: ErrorCode, message: String, data: Option<Value>) -> Self {
        Self {
            code,
            message,
            data,
        }
    }
    pub fn parse_error(message: String, data: Option<Value>) -> Self {
        Self::new(ErrorCode::PARSE_ERROR, message, data)
    }
    pub fn invalid_request(message: String, data: Option<Value>) -> Self {
        Self::new(ErrorCode::INVALID_REQUEST, message, data)
    }
    pub fn method_not_found(message: String, data: Option<Value>) -> Self {
        Self::new(ErrorCode::METHOD_NOT_FOUND, message, data)
    }
    pub fn invalid_params(message: String, data: Option<Value>) -> Self {
        Self::new(ErrorCode::INVALID_PARAMS, message, data)
    }
    pub fn internal_error(message: String, data: Option<Value>) -> Self {
        Self::new(ErrorCode::INTERNAL_ERROR, message, data)
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
#[serde(untagged)]
pub enum JsonRpcMessage<Req = Request, Resp = DefaultResponse, Noti = Notification> {
    Request(JsonRpcRequest<Req>),
    Response(JsonRpcResponse<Resp>),
    Notification(JsonRpcNotification<Noti>),
    Error(JsonRpcError),
}

impl<Req, Resp, Noti> JsonRpcMessage<Req, Resp, Noti> {
    pub fn into_message(self) -> Message<Req, Resp, Noti> {
        match self {
            JsonRpcMessage::Request(JsonRpcRequest { id, request, .. }) => {
                Message::Request(request, id)
            }
            JsonRpcMessage::Response(JsonRpcResponse { id, result, .. }) => {
                Message::Response(result, id)
            }
            JsonRpcMessage::Notification(JsonRpcNotification { notification, .. }) => {
                Message::Notification(notification)
            }
            JsonRpcMessage::Error(JsonRpcError { id, error, .. }) => Message::Error(error, id),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Message<Req = Request, Resp = DefaultResponse, Noti = Notification> {
    Request(Req, RequestId),
    Response(Resp, RequestId),
    Error(ErrorData, RequestId),
    Notification(Noti),
}

impl<Req, Resp, Noti> Message<Req, Resp, Noti> {
    pub fn into_notification(self) -> Option<Noti> {
        match self {
            Message::Notification(notification) => Some(notification),
            _ => None,
        }
    }
    pub fn into_response(self) -> Option<(Resp, RequestId)> {
        match self {
            Message::Response(result, id) => Some((result, id)),
            _ => None,
        }
    }
    pub fn into_request(self) -> Option<(Req, RequestId)> {
        match self {
            Message::Request(request, id) => Some((request, id)),
            _ => None,
        }
    }
    pub fn into_error(self) -> Option<(ErrorData, RequestId)> {
        match self {
            Message::Error(error, id) => Some((error, id)),
            _ => None,
        }
    }
    pub fn into_json_rpc_message(self) -> JsonRpcMessage<Req, Resp, Noti> {
        match self {
            Message::Request(request, id) => JsonRpcMessage::Request(JsonRpcRequest {
                jsonrpc: JsonRpcVersion2_0,
                id,
                request,
            }),
            Message::Response(result, id) => JsonRpcMessage::Response(JsonRpcResponse {
                jsonrpc: JsonRpcVersion2_0,
                id,
                result,
            }),
            Message::Error(error, id) => JsonRpcMessage::Error(JsonRpcError {
                jsonrpc: JsonRpcVersion2_0,
                id,
                error,
            }),
            Message::Notification(notification) => {
                JsonRpcMessage::Notification(JsonRpcNotification {
                    jsonrpc: JsonRpcVersion2_0,
                    notification,
                })
            }
        }
    }
}

/// # Empty result
/// A response that indicates success but carries no data.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Default)]
pub struct EmptyResult {}

impl From<()> for EmptyResult {
    fn from(_value: ()) -> Self {
        EmptyResult {}
    }
}

impl From<EmptyResult> for () {
    fn from(_value: EmptyResult) {}
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CancelledNotificationParam {
    request_id: RequestId,
    reason: Option<String>,
}

const_string!(CancelledNotificationMethod = "notifications/cancelled");

/// # Cancellation
/// This notification can be sent by either side to indicate that it is cancelling a previously-issued request.
///
/// The request SHOULD still be in-flight, but due to communication latency, it is always possible that this notification MAY arrive after the request has already finished.
///
/// This notification indicates that the result will be unused, so any associated processing SHOULD cease.
///
/// A client MUST NOT attempt to cancel its `initialize` request.
pub type CancelledNotification =
    Notification<CancelledNotificationMethod, CancelledNotificationParam>;

const_string!(InitializeResultMethod = "initialize");
/// # Initialization
/// This request is sent from the client to the server when it first connects, asking it to begin initialization.
pub type InitializeRequest = Request<InitializeResultMethod, InitializeRequestParam>;

const_string!(InitializedNotificationMethod = "notifications/initialized");
/// This notification is sent from the client to the server after initialization has finished.
pub type InitializedNotification = Notification<InitializedNotificationMethod, ()>;
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct InitializeRequestParam {
    pub protocol_version: String,
    pub capabilities: ClientCapabilities,
    pub server_info: Implementation,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct InitializeResult {
    pub protocol_version: LatestProtocolVersion,
    pub capabilities: ServerCapabilities,
    pub server_info: Implementation,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instructions: Option<String>,
}

pub type ExperimentalCapabilities = BTreeMap<String, JsonObject>;

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RootsCapabilities {
    list_changed: Option<bool>,
}
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct ClientCapabilities {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub experimental: Option<ExperimentalCapabilities>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub roots: Option<RootsCapabilities>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sampling: Option<JsonObject>,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct ServerCapabilities {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub experimental: Option<ExperimentalCapabilities>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub logging: Option<JsonObject>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompts: Option<PromptsCapability>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resources: Option<ResourcesCapability>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<ToolsCapability>,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct Implementation {
    pub name: String,
    pub version: String,
}

impl Default for Implementation {
    fn default() -> Self {
        Self::from_build_env()
    }
}

impl Implementation {
    pub fn from_build_env() -> Self {
        Implementation {
            name: env!("CARGO_CRATE_NAME").to_owned(),
            version: env!("CARGO_PKG_VERSION").to_owned(),
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PromptsCapability {
    pub list_changed: Option<bool>,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ResourcesCapability {
    pub subscribe: Option<bool>,
    pub list_changed: Option<bool>,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ToolsCapability {
    pub list_changed: Option<bool>,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PaginatedRequestParam {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
}

const_string!(PingRequestMethod = "ping");
pub type PingRequest = Request<PingRequestMethod, ()>;

const_string!(ProgressNotificationMethod = "notifications/progress");
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ProgressNotificationParam {
    pub progress_token: ProgressToken,
    pub progress: i32,
    pub total: i32,
}

pub type ProgressNotification = Notification<ProgressNotificationMethod, ProgressNotificationParam>;

pub type Cursor = String;

macro_rules! paginated_result {
    ($t:ident {
        $i_item: ident: $t_item: ty
    }) => {
        #[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
        #[serde(rename_all = "camelCase")]
        pub struct $t {
            #[serde(skip_serializing_if = "Option::is_none")]
            pub next_cursor: Option<Cursor>,
            pub $i_item: $t_item,
        }
    };
}

const_string!(ListResourcesRequestMethod = "resources/list");
pub type ListResourcesRequest = Request<ListResourcesRequestMethod, PaginatedRequestParam>;
paginated_result!(ListResourcesResult {
    resources: Vec<Resource>
});

const_string!(ListResourceTemplatesRequestMethod = "resources/templates/list");
pub type ListResourceTemplatesRequest =
    Request<ListResourceTemplatesRequestMethod, PaginatedRequestParam>;
paginated_result!(ListResourceTemplatesResult {
    resource_templates: Vec<Resource>
});

const_string!(ReadResourceRequestMethod = "resources/read");
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ReadResourceRequestParam {
    pub uri: String,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct ReadResourceResult {
    pub contents: Vec<ResourceContents>,
}

pub type ReadResourceRequest = Request<ReadResourceRequestMethod, ReadResourceRequestParam>;

const_string!(ResourceListChangedNotificationMethod = "notifications/resources/list_changed");
pub type ResourceListChangedNotification = Notification<ResourceListChangedNotificationMethod, ()>;

const_string!(SubscribeRequestMethod = "resources/subscribe");
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SubscribeRequestParam {
    pub uri: String,
}
pub type SubscribeRequest = Request<SubscribeRequestMethod, SubscribeRequestParam>;

const_string!(UnsubscribeRequestMethod = "resources/unsubscribe");
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct UnsubscribeRequestParam {
    pub uri: String,
}
pub type UnsubscribeRequest = Request<UnsubscribeRequestMethod, UnsubscribeRequestParam>;

const_string!(ResourceUpdatedNotificationMethod = "notifications/resources/updated");
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ResourceUpdatedNotificationParam {
    pub uri: String,
}
pub type ResourceUpdatedNotification =
    Notification<ResourceUpdatedNotificationMethod, ResourceUpdatedNotificationParam>;

const_string!(ListPromptsRequestMethod = "prompts/list");
pub type ListPromptsRequest = Request<ListPromptsRequestMethod, PaginatedRequestParam>;
paginated_result!(ListPromptsResult {
    prompts: Vec<Prompt>
});

const_string!(GetPromptRequestMethod = "prompts/get");
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct GetPromptRequestParam {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub arguments: Option<BTreeMap<String, String>>,
}
pub type GetPromptRequest = Request<GetPromptRequestMethod, GetPromptRequestParam>;

const_string!(PromptListChangedNotificationMethod = "notifications/prompts/list_changed");
pub type PromptListChangedNotification = Notification<PromptListChangedNotificationMethod, ()>;

const_string!(ToolListChangedNotificationMethod = "notifications/tools/list_changed");
pub type ToolListChangedNotification = Notification<ToolListChangedNotificationMethod, ()>;
// 日志相关
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum LoggingLevel {
    Debug,
    Info,
    Notice,
    Warning,
    Error,
    Critical,
    Alert,
    Emergency,
}

const_string!(SetLevelRequestMethod = "logging/setLevel");
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SetLevelRequestParam {
    pub level: LoggingLevel,
}
pub type SetLevelRequest = Request<SetLevelRequestMethod, SetLevelRequestParam>;

const_string!(LoggingMessageNotificationMethod = "notifications/message");
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LoggingMessageNotificationParam {
    pub level: LoggingLevel,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub logger: Option<String>,
    pub data: Value,
}
pub type LoggingMessageNotification =
    Notification<LoggingMessageNotificationMethod, LoggingMessageNotificationParam>;

const_string!(CreateMessageRequestMethod = "sampling/createMessage");
pub type CreateMessageRequest = Request<CreateMessageRequestMethod, CreateMessageRequestParam>;

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct SamplingMessage {
    pub role: Role,
    pub content: Content,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CreateMessageRequestParam {
    pub messages: Vec<SamplingMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_preferences: Option<ModelPreferences>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system_prompt: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub include_context: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    pub max_tokens: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stop_sequences: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Value>,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ModelPreferences {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hints: Option<Vec<ModelHint>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cost_priority: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub speed_priority: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub intelligence_priority: Option<f32>,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct ModelHint {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CompleteRequestParam {
    pub r#ref: Reference,
    pub argument: ArgumentInfo,
}

pub type CompleteRequest = Request<CompleteRequestMethod, CompleteRequestParam>;

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CompletionInfo {
    pub values: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub has_more: Option<bool>,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CompleteResult {
    pub completion: CompletionInfo,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
#[serde(tag = "type")]
pub enum Reference {
    #[serde(rename = "ref/resource")]
    Resource(ResourceReference),
    #[serde(rename = "ref/prompt")]
    Prompt(PromptReference),
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct ResourceReference {
    pub uri: String,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct PromptReference {
    pub name: String,
}

const_string!(CompleteRequestMethod = "completion/complete");
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ArgumentInfo {
    pub name: String,
    pub value: String,
}

// 根目录相关
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct Root {
    pub uri: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

const_string!(ListRootsRequestMethod = "roots/list");
pub type ListRootsRequest = Request<ListRootsRequestMethod, ()>;

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ListRootsResult {
    pub roots: Vec<Root>,
}

const_string!(RootsListChangedNotificationMethod = "notifications/roots/list_changed");
pub type RootsListChangedNotification = Notification<RootsListChangedNotificationMethod, ()>;

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CallToolResult {
    pub content: Vec<Content>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_error: Option<bool>,
}

impl CallToolResult {
    pub fn success(content: Vec<Content>) -> Self {
        CallToolResult {
            content,
            is_error: Some(false),
        }
    }
}

const_string!(ListToolsRequestMethod = "tools/list");
pub type ListToolsRequest = Request<ListToolsRequestMethod, PaginatedRequestParam>;
paginated_result!(
    ListToolsResult {
        tools: Vec<Tool>
    }
);

const_string!(CallToolRequestMethod = "tools/call");
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CallToolRequestParam {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub arguments: Option<BTreeMap<String, Value>>,
}

pub type CallToolRequest = Request<CallToolRequestMethod, CallToolRequestParam>;

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CreateMessageResult {
    pub model: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stop_reason: Option<String>,
    #[serde(flatten)]
    pub message: SamplingMessage,
}

impl CreateMessageResult {
    pub const STOP_REASON_END_TURN: &str = "endTurn";
    pub const STOP_REASON_END_SEQUENCE: &str = "stopSequence";
    pub const STOP_REASON_END_MAX_TOKEN: &str = "maxTokens";
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct GetPromptResult {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub messages: Vec<PromptMessage>,
}

macro_rules! ts_union {
    (
        export type $U: ident =
            $(|)?$($V: ident)|*;
    ) => {
        #[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
        #[serde(untagged)]
        pub enum $U {
            $($V($V),)*
        }
    };
}

ts_union!(
    export type ClientRequest =
    | PingRequest
    | InitializeRequest
    | CompleteRequest
    | SetLevelRequest
    | GetPromptRequest
    | ListPromptsRequest
    | ListResourcesRequest
    | ListResourceTemplatesRequest
    | ReadResourceRequest
    | SubscribeRequest
    | UnsubscribeRequest
    | CallToolRequest
    | ListToolsRequest;
);

ts_union!(
    export type ClientNotification =
    | CancelledNotification
    | ProgressNotification
    | InitializedNotification
    | RootsListChangedNotification;
);

ts_union!(
    export type ClientResult = EmptyResult | CreateMessageResult | ListRootsResult;
);

pub type ClientJsonRpcMessage = JsonRpcMessage<ClientRequest, ClientResult, ClientNotification>;
pub type ClientMessage = Message<ClientRequest, ClientResult, ClientNotification>;

ts_union!(
    export type ServerRequest =
    | PingRequest
    | CreateMessageRequest
    | ListRootsRequest;
);

ts_union!(
    export type ServerNotification =
    | CancelledNotification
    | ProgressNotification
    | LoggingMessageNotification
    | ResourceUpdatedNotification
    | ResourceListChangedNotification
    | ToolListChangedNotification
    | PromptListChangedNotification;
);

ts_union!(
    export type ServerResult =
    | EmptyResult
    | InitializeResult
    | CompleteResult
    | GetPromptResult
    | ListPromptsResult
    | ListResourcesResult
    | ListResourceTemplatesResult
    | ReadResourceResult
    | CallToolResult
    | ListToolsResult;
);

impl ServerResult {
    pub fn empty(_: ()) -> ServerResult {
        ServerResult::EmptyResult(EmptyResult {})
    }
}

pub type ServerJsonRpcMessage = JsonRpcMessage<ServerRequest, ServerResult, ServerNotification>;
pub type ServerMessage = Message<ServerRequest, ServerResult, ServerNotification>;

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_notification_serde() {
        let raw = json!( {
            "jsonrpc": JsonRpcVersion2_0,
            "method": InitializedNotificationMethod,
        });
        let message: ClientJsonRpcMessage =
            serde_json::from_value(raw.clone()).expect("invalid notification");
        let message = message.into_message();
        match &message {
            ClientMessage::Notification(ClientNotification::InitializedNotification(_n)) => {}
            _ => panic!("Expected Notification"),
        }
        let json = serde_json::to_value(message.into_json_rpc_message()).expect("valid json");
        assert_eq!(json, raw);
    }

    #[test]
    fn test_request_conversion() {
        let raw = json!( {
            "jsonrpc": JsonRpcVersion2_0,
            "id": 1,
            "method": "request",
            "params": {"key": "value"},
        });
        let message: JsonRpcMessage = serde_json::from_value(raw.clone()).expect("invalid request");

        match &message {
            JsonRpcMessage::Request(r) => {
                assert_eq!(r.id, RequestId::Number(1));
                assert_eq!(r.request.method, "request");
                assert_eq!(
                    &r.request.params.as_ref().unwrap().inner,
                    json!({"key": "value"})
                        .as_object()
                        .expect("should be an object")
                );
            }
            _ => panic!("Expected Request"),
        }
        let json = serde_json::to_value(&message).expect("valid json");
        assert_eq!(json, raw);
    }
}
