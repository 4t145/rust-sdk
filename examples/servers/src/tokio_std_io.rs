use anyhow::Result;
use mcp_core::transport::io::async_rw;
use mcp_server::{ServerHandlerService, serve_server};
use tokio::io::{stdin, stdout};
use tracing_subscriber::{self, EnvFilter};
mod common;
/// npx @modelcontextprotocol/inspector cargo run -p mcp-server
#[tokio::main]
async fn main() -> Result<()> {
    // Initialize the tracing subscriber with file and stdout logging
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env().add_directive(tracing::Level::DEBUG.into()))
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .init();

    tracing::info!("Starting MCP server");

    // Create an instance of our counter router
    let service = ServerHandlerService::new(common::counter::Counter::new());
    let transport = async_rw(stdin(), stdout());
    serve_server(service, transport).await.inspect_err(|e| {
        tracing::error!("serving error: {:?}", e);
    })?;

    Ok(())
}
