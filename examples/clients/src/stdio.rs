use anyhow::Result;
use mcp_client::handler::ClientHandlerService;
use mcp_client::serve_client;
use mcp_client::transport::child_process::child_process;
use mcp_core::schema::CallToolRequestParam;

use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
mod common;
use common::simple_client::SimpleClient;

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| format!("info,{}=debug", env!("CARGO_CRATE_NAME")).into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();
    let (mut process, transport) = child_process(
        tokio::process::Command::new("uvx")
            .arg("mcp-server-git")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()?,
    )?;
    let client = ClientHandlerService::new(SimpleClient::default());

    let running_service = serve_client(client, transport).await.inspect_err(|e| {
        tracing::error!("client error: {:?}", e);
    })?;

    // Initialize
    let server_info = running_service.peer().info();
    tracing::info!("Connected to server: {server_info:#?}");

    // List tools
    let tools = running_service
        .peer()
        .list_tools(Default::default())
        .await?;
    tracing::info!("Available tools: {tools:#?}");

    // Call tool 'git_status' with arguments = {"repo_path": "."}
    let tool_result = running_service
        .peer()
        .call_tool(CallToolRequestParam {
            name: "git_status".into(),
            arguments: serde_json::json!({ "repo_path": "." }).as_object().cloned(),
        })
        .await?;
    tracing::info!("Tool result: {tool_result:#?}");

    process.kill().await?;
    Ok(())
}
