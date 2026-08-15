//! SWAI — Inter-Process Communication (IPC) subsystem.

pub mod client;
pub mod handler;
pub mod protocol;
pub mod server;
#[cfg(test)]
mod tests_protocol;
#[cfg(test)]
mod tests_server;

pub use client::{ipc_send, IpcClientError};
pub use handler::{dispatch_action, handle_request_sync, resolve_cycle_model_id, send_response_sync};
pub use protocol::{ActionRequest, ActionResponse, IpcState};
pub use server::{cleanup_stale_socket, config_dir, socket_path, start_ipc_server, IpcServerHandle};
