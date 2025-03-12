use anyhow::Result;
use mcp_server::{ServerHandlerService, RoleServer, serve, };
use tokio::io::{stdin, stdout};
use tracing_appender::rolling::{RollingFileAppender, Rotation};
use tracing_subscriber::{self, EnvFilter};
use mcp_core::transport::tokio_io::{tokio_rw};
mod common;

#[tokio::main]
async fn main() -> Result<()> {
    // Set up file appender for logging
    let file_appender = RollingFileAppender::new(Rotation::DAILY, "logs", "mcp-server.log");

    // Initialize the tracing subscriber with file and stdout logging
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env().add_directive(tracing::Level::INFO.into()))
        .with_writer(file_appender)
        .with_target(false)
        .with_thread_ids(true)
        .with_file(true)
        .with_line_number(true)
        .init();

    tracing::info!("Starting MCP server");

    // Create an instance of our counter router
    let service = ServerHandlerService::new(common::counter::Counter::new());
    let transport = tokio_rw(stdin(), stdout());
    tracing::info!("Server initialized and ready to handle requests");
    let transport = serve(service, transport).await;

    Ok(())
}
