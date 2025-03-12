use anyhow::Result;
use mcp_core::transport::tokio_io::tokio_rw;
use mcp_server::{ServerHandlerService, serve};
use tokio::io::{stdin, stdout};
use tracing_subscriber::{self, EnvFilter};
mod common;

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
    let transport = tokio_rw(stdin(), stdout());
    serve(service, transport).await.inspect_err(|e| {
        tracing::error!("serving error: {:?}", e);
    })?;

    Ok(())
}
