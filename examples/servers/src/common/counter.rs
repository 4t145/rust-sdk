use std::{future::Future, pin::Pin, sync::Arc};

use mcp_core::{
    handler::{PromptError, ResourceError},
    prompt::{Prompt, PromptArgument, PromptMessage, PromptMessageContent, PromptMessageRole},
    schema::*,
    Content, Resource, ResourceContents, Role, Tool, ToolError,
};
use mcp_server::Handler;
use serde_json::Value;
use tokio::sync::Mutex;

#[derive(Clone)]
pub struct Counter {
    counter: Arc<Mutex<i32>>,
}

impl Counter {
    pub fn new() -> Self {
        Self {
            counter: Arc::new(Mutex::new(0)),
        }
    }

    async fn increment(&self) -> i32 {
        let mut counter = self.counter.lock().await;
        *counter += 1;
        *counter
    }

    async fn decrement(&self) -> i32 {
        let mut counter = self.counter.lock().await;
        *counter -= 1;
        *counter
    }

    async fn get_value(&self) -> i32 {
        let counter = self.counter.lock().await;
        *counter
    }

    fn _create_resource_text(&self, uri: &str, name: &str) -> Resource {
        Resource::new(uri, Some("text/plain".to_string()), Some(name.to_string())).unwrap()
    }
}

impl Handler for Counter {
    async fn initialize(
        &self,
        _request: mcp_core::schema::InitializeRequestParam,
    ) -> Result<mcp_core::schema::InitializeResult, mcp_core::error::Error> {
        Ok(mcp_core::schema::InitializeResult {
            protocol_version: mcp_core::schema::LatestProtocolVersion,
            capabilities: ServerCapabilities {
                experimental: None,
                logging: None,
                prompts: None,
                resources: None,
                tools: Some(ToolsCapability {
                    list_changed: None,
                }),
            },
            server_info: Implementation::from_build_env(),
            instructions: Some("This server provides a counter tool that can increment and decrement values. The counter starts at 0 and can be modified using the 'increment' and 'decrement' tools. Use 'get_value' to check the current count.".to_string()),
        })
    }
    async fn list_tools(
        &self,
        _request: mcp_core::schema::PaginatedRequestParam,
    ) -> Result<mcp_core::schema::ListToolsResult, mcp_core::error::Error> {
        let tools = vec![
            Tool::new(
                "increment".to_string(),
                "Increment the counter by 1".to_string(),
                serde_json::json!({
                    "type": "object",
                    "properties": {},
                    "required": []
                }),
            ),
            Tool::new(
                "decrement".to_string(),
                "Decrement the counter by 1".to_string(),
                serde_json::json!({
                    "type": "object",
                    "properties": {},
                    "required": []
                }),
            ),
            Tool::new(
                "get_value".to_string(),
                "Get the current counter value".to_string(),
                serde_json::json!({
                    "type": "object",
                    "properties": {},
                    "required": []
                }),
            ),
        ];
        Ok(ListToolsResult {
            next_cursor: None,
            tools,
        })
    }

    async fn call_tool(
        &self,
        CallToolRequestParam { name, arguments: _ }: CallToolRequestParam,
    ) -> Result<CallToolResult, mcp_core::error::Error> {
        match name.as_str() {
            "increment" => {
                let value = self.increment().await;
                Ok(CallToolResult::success(vec![Content::text(
                    value.to_string(),
                )]))
            }
            "decrement" => {
                let value = self.decrement().await;
                Ok(CallToolResult::success(vec![Content::text(
                    value.to_string(),
                )]))
            }
            "get_value" => {
                let value = self.get_value().await;
                Ok(CallToolResult::success(vec![Content::text(
                    value.to_string(),
                )]))
            }
            _ => Err(mcp_core::error::Error::not_found()),
        }
    }

    async fn list_resources(
        &self,
        request: mcp_core::schema::PaginatedRequestParam,
    ) -> Result<mcp_core::schema::ListResourcesResult, mcp_core::error::Error> {
        Ok(mcp_core::schema::ListResourcesResult {
            resources: vec![
                self._create_resource_text("str:////Users/to/some/path/", "cwd"),
                self._create_resource_text("memo://insights", "memo-name"),
            ],
            next_cursor: None,
        })
    }

    async fn read_resource(
        &self,
        ReadResourceRequestParam { uri }: ReadResourceRequestParam,
    ) -> Result<ReadResourceResult, mcp_core::error::Error> {
        match uri.as_str() {
            "str:////Users/to/some/path/" => {
                let cwd = "/Users/to/some/path/";
                Ok(ReadResourceResult {
                    contents: vec![ResourceContents::text(cwd, uri)],
                })
            }
            "memo://insights" => {
                let memo = "Business Intelligence Memo\n\nAnalysis has revealed 5 key insights ...";
                Ok(ReadResourceResult {
                    contents: vec![ResourceContents::text(memo, uri)],
                })
            }
            _ => Err(mcp_core::error::Error::not_found()),
        }
    }

    async fn list_prompts(
        &self,
        _request: PaginatedRequestParam,
    ) -> Result<ListPromptsResult, mcp_core::error::Error> {
        Ok(ListPromptsResult {
            next_cursor: None,
            prompts: vec![Prompt::new(
                "example_prompt",
                Some("This is an example prompt that takes one required agrument, message"),
                Some(vec![PromptArgument {
                    name: "message".to_string(),
                    description: Some("A message to put in the prompt".to_string()),
                    required: Some(true),
                }]),
            )],
        })
    }

    async fn get_prompt(
        &self,
        GetPromptRequestParam { name, arguments }: GetPromptRequestParam,
    ) -> Result<GetPromptResult, mcp_core::error::Error> {
        match name.as_str() {
            "example_prompt" => {
                let prompt = "This is an example prompt with your message here: '{message}'";
                Ok(GetPromptResult {
                    description: None,
                    messages: vec![PromptMessage {
                        role: PromptMessageRole::User,
                        content: PromptMessageContent::text(prompt),
                    }],
                })
            }
            _ => Err(mcp_core::error::Error::not_found()),
        }
    }
}
