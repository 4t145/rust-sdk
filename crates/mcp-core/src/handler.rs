use std::{borrow::Cow, future::Future, marker::PhantomData, ops::Deref, pin::Pin};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::Value;
use thiserror::Error;

use crate::Content;

#[non_exhaustive]
#[derive(Error, Debug, Clone, Deserialize, Serialize, PartialEq)]
pub enum ToolError {
    #[error("Invalid parameters: {0}")]
    InvalidParameters(String),
    #[error("Execution failed: {0}")]
    ExecutionError(String),
    #[error("Schema error: {0}")]
    SchemaError(String),
    #[error("Tool not found: {0}")]
    NotFound(String),
}

impl ToolError {
    pub fn execution<E: ToString>(e: E) -> Self {
        Self::ExecutionError(e.to_string())
    }
}

pub type ToolResult<T> = std::result::Result<T, ToolError>;

#[derive(Error, Debug)]
pub enum ResourceError {
    #[error("Execution failed: {0}")]
    ExecutionError(String),
    #[error("Resource not found: {0}")]
    NotFound(String),
}

impl ResourceError {
    pub fn execution<E: ToString>(e: E) -> Self {
        Self::ExecutionError(e.to_string())
    }
}
#[derive(Error, Debug)]
pub enum PromptError {
    #[error("Invalid parameters: {0}")]
    InvalidParameters(String),
    #[error("Internal error: {0}")]
    InternalError(String),
    #[error("Prompt not found: {0}")]
    NotFound(String),
}

#[derive(Error, Debug)]
pub enum NotificationError {
    #[error("Invalid parameters: {0}")]
    InvalidParameters(String),
    #[error("Internal error: {0}")]
    InternalError(String),
    #[error("Prompt not found: {0}")]
    NotFound(String),
}

type NotificationResult = Result<(), NotificationError>;

/// Trait for implementing MCP tools
pub trait ToolHandler: Send + Sync {
    /// The name of the tool
    fn name(&self) -> &'static str;

    /// A description of what the tool does
    fn description(&self) -> &'static str;

    /// JSON schema describing the tool's parameters
    fn schema(&self) -> Value;

    /// Execute the tool with the given parameters
    fn call(
        &self,
        params: Value,
    ) -> Pin<Box<dyn Future<Output = ToolResult<Vec<Content>>> + Send + '_>>;
}

/// Trait for implementing MCP tools with specified types
pub trait TypedToolHandler<A = ()>: Send + Sync {
    #[cfg(feature = "default_json_schema")]
    type Params: DeserializeOwned + JsonSchema;

    #[cfg(not(feature = "default_json_schema"))]
    type Params: DeserializeOwned;
    /// The name of the tool
    fn name(&self) -> &'static str;

    /// A description of what the tool does
    fn description(&self) -> &'static str;

    /// JSON schema describing the tool's parameters
    #[cfg(feature = "default_json_schema")]
    fn schema(&self) -> Value {
        serde_json::to_value(schemars::schema_for!(Self::Params))
            .expect("json schema should always be a valid json object")
    }

    #[cfg(not(feature = "default_json_schema"))]
    fn schema(&self) -> Value;

    /// Execute the tool with the given parameters
    fn call(&self, params: Self::Params) -> impl Future<Output = ToolResult<Vec<Content>>> + Send;
}

pub struct FunctionHandler<F, P, Fut> {
    name: &'static str,
    description: &'static str,
    f: F,
    _params: PhantomData<fn(P) -> Fut>,
}

impl<F, P, Fut> FunctionHandler<F, P, Fut> {
    pub const fn new(name: &'static str, description: &'static str, f: F) -> Self {
        Self {
            name,
            description,
            f,
            _params: PhantomData,
        }
    }
}

impl<P, F, Fut> TypedToolHandler for FunctionHandler<F, P, Fut>
where
    F: Fn(P) -> Fut + Send + Sync,
    Fut: Future<Output = ToolResult<Vec<Content>>> + Send,
    P: DeserializeOwned + JsonSchema + Send + Sync,
{
    type Params = P;

    fn name(&self) -> &'static str {
        self.name
    }

    fn description(&self) -> &'static str {
        self.description
    }

    async fn call(&self, params: Self::Params) -> ToolResult<Vec<Content>> {
        (self.f)(params).await
    }
}

#[diagnostic::do_not_recommend]
impl<H: TypedToolHandler> ToolHandler for H {
    fn name(&self) -> &'static str {
        TypedToolHandler::name(self)
    }

    fn description(&self) -> &'static str {
        TypedToolHandler::description(self)
    }

    fn schema(&self) -> Value {
        TypedToolHandler::schema(self)
    }

    fn call(
        &self,
        params: Value,
    ) -> Pin<Box<dyn Future<Output = ToolResult<Vec<Content>>> + Send + '_>> {
        Box::pin(async {
            let input = serde_json::from_value(params)
                .map_err(|e| ToolError::InvalidParameters(e.to_string()))?;
            let result = TypedToolHandler::call(self, input)
                .await?
                .into_iter()
                .collect();
            Ok(result)
        })
    }
}

pub struct DynToolHandler(Box<dyn ToolHandler>);

impl Deref for DynToolHandler {
    type Target = dyn ToolHandler;
    fn deref(&self) -> &Self::Target {
        self.0.as_ref()
    }
}

impl DynToolHandler {
    /// Convert from a [`TypedToolHandler`]
    pub fn new<H: ToolHandler + 'static>(handler: H) -> Self {
        Self(Box::new(handler))
    }
    pub fn new_boxed(handler: Box<dyn ToolHandler>) -> Self {
        Self(handler)
    }
}

/// Trait for implementing MCP resources
pub trait ResourceTemplateHandler: Send + Sync + 'static {
    /// The URL template for this resource
    fn template() -> &'static str;

    /// JSON schema describing the resource parameters
    fn schema() -> Value;

    /// Get the resource value
    fn get(&self, params: Value) -> impl Future<Output = ToolResult<String>>;
}

pub trait NotificationHandler: Send + Sync + 'static {
    fn method(&self) -> Cow<'static, str>;
    /// JSON schema describing the resource parameters
    fn schema() -> Value;
    fn notify(&self, method: String) -> impl Future<Output = NotificationResult> + Send + '_;
}

pub trait TypedNotificationHandler: Send + Sync + 'static {
    fn method(&self) -> Cow<'static, str>; 
    fn notify(&self, method: String) -> impl Future<Output = NotificationResult> + Send + '_;
}

/// Helper function to generate JSON schema for a type
pub fn generate_schema<T: JsonSchema>() -> ToolResult<Value> {
    let schema = schemars::schema_for!(T);
    serde_json::to_value(schema).map_err(|e| ToolError::SchemaError(e.to_string()))
}


