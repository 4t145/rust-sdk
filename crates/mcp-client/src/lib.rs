// pub mod client;
// pub mod service;
// pub mod transport;

// pub use client::{ClientCapabilities, ClientInfo, Error, McpClient, McpClientTrait};
// pub use service::McpService;
// pub use transport::{SseTransport, StdioTransport, Transport, TransportHandle};


pub mod handler;
// pub use handler::{Handler, ServerHandlerService};
pub use mcp_core::service::{PeerProxy, RoleClient, Service, ServiceError, serve};
pub use mcp_core::error::Error as McpError;
pub use mcp_core::transport;
pub use mcp_core::schema;
pub use mcp_core as core;