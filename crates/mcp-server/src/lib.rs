pub mod handler;
pub use handler::{ServerHandler, ServerHandlerService};
pub use mcp_core::service::{PeerProxy, RoleServer, Service, ServiceError, serve};
pub use mcp_core::error::Error as McpError;
pub use mcp_core::transport;
pub use mcp_core::schema;
pub use mcp_core as core;