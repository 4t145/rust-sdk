pub mod handler;
pub use handler::{Handler, ServerHandlerService};
pub use mcp_core::service::{PeerProxy, RoleServer, Service, ServiceError, serve};
