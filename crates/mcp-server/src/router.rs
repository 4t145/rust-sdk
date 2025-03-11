use std::{
    future::Future,
    pin::Pin,
    task::{Context, Poll},
};

type PromptFuture = Pin<Box<dyn Future<Output = Result<String, PromptError>> + Send + 'static>>;

use mcp_core::{
    ResourceContents,
    content::Content,
    handler::{PromptError, ResourceError, ToolError},
    prompt::{Prompt, PromptMessage, PromptMessageRole},
    schema::{
        CallToolResult, GetPromptResult, Implementation, InitializeResult, JsonRpcRequest,
        JsonRpcResponse, ListPromptsResult, ListResourcesResult, ListToolsResult,
        PromptsCapability, ReadResourceResult, ResourcesCapability, ServerCapabilities,
        ToolsCapability,
    },
};
use serde_json::Value;
use tower_service::Service;

use crate::{BoxError, RouterError};

/// Builder for configuring and constructing capabilities
pub struct CapabilitiesBuilder {
    tools: Option<ToolsCapability>,
    prompts: Option<PromptsCapability>,
    resources: Option<ResourcesCapability>,
}

impl Default for CapabilitiesBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl CapabilitiesBuilder {
    pub fn new() -> Self {
        Self {
            tools: None,
            prompts: None,
            resources: None,
        }
    }

    /// Add multiple tools to the router
    pub fn with_tools(mut self, list_changed: bool) -> Self {
        self.tools = Some(ToolsCapability {
            list_changed: Some(list_changed),
        });
        self
    }

    /// Enable prompts capability
    pub fn with_prompts(mut self, list_changed: bool) -> Self {
        self.prompts = Some(PromptsCapability {
            list_changed: Some(list_changed),
        });
        self
    }

    /// Enable resources capability
    pub fn with_resources(mut self, subscribe: bool, list_changed: bool) -> Self {
        self.resources = Some(ResourcesCapability {
            subscribe: Some(subscribe),
            list_changed: Some(list_changed),
        });
        self
    }

    /// Build the router with automatic capability inference
    pub fn build(self) -> ServerCapabilities {
        // Create capabilities based on what's configured
        ServerCapabilities {
            tools: self.tools,
            prompts: self.prompts,
            resources: self.resources,
            experimental: None,
            logging: None,
        }
    }
}
