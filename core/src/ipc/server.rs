use std::io::{self};
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};

pub use super::handler::handle_request_sync;
use super::protocol::IpcState;

/// The directory where SWAI stores its runtime files.
pub fn config_dir() -> PathBuf {
    if let Ok(home) = std::env::var("HOME") {
        PathBuf::from(home).join(".config").join("swai")
    } else {
        PathBuf::from(".config/swai")
    }
}

/// The Unix socket path used by the IPC server.
pub fn socket_path() -> PathBuf {
    config_dir().join("swai.sock")
}

/// Remove a stale socket file if it exists (e.g. from a crashed previous run).
pub fn cleanup_stale_socket(path: &Path) {
    if path.exists() {
        match std::fs::remove_file(path) {
            Ok(()) => debug!("removed stale IPC socket at {:?}", path),
            Err(e) => warn!("failed to remove stale IPC socket {:?}: {}", path, e),
        }
    }
}

// ---------------------------------------------------------------------------
// IPC Server
// ---------------------------------------------------------------------------

/// Handle to the running IPC server.
///
/// Drop this handle to stop the background listener task and close the socket.
pub struct IpcServerHandle {
    /// Channel receiver for the background listener task.
    _receiver: mpsc::Receiver<()>,
}

impl IpcServerHandle {
    /// Stop the IPC server, closing the socket and cancelling the listener.
    pub fn stop(self) {
        info!("stopping IPC server");
        // Dropping `_receiver` cancels the spawned task.
        drop(self);
        // Clean up the socket file.
        let path = socket_path();
        if path.exists() {
            let _ = std::fs::remove_file(&path);
        }
    }
}

/// Start the IPC server in a background tokio task.
///
/// Returns an `IpcServerHandle` that can be used to stop the server. The server
/// binds to `~/.config/swai/swai.sock` (cleaning up any stale socket first).
pub fn start_ipc_server(state: Arc<IpcState>) -> Result<IpcServerHandle, io::Error> {
    let path = socket_path();

    // Ensure the config directory exists.
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    // Remove any stale socket from a previous crash.
    cleanup_stale_socket(&path);

    // Bind the Unix domain socket listener.
    let listener = UnixListener::bind(&path)?;
    listener.set_nonblocking(false)?;

    info!("IPC server listening on {:?}", path);

    let (tx, rx) = mpsc::channel::<()>(1);

    tokio::spawn(async move {
        loop {
            match listener.accept() {
                Ok((stream, addr)) => {
                    debug!("IPC client connected from {:?}", addr);
                    // Handle the request in a sub-task using blocking I/O.
                    let state = Arc::clone(&state);
                    tokio::task::spawn_blocking(move || {
                        if let Err(e) = handle_request_sync(stream, &state) {
                            error!("IPC request handler error: {}", e);
                        }
                        debug!("IPC client disconnected");
                    });
                }
                Err(e) => {
                    // Broken pipe / shutdown — exit the loop.
                    if e.kind() == io::ErrorKind::BrokenPipe {
                        info!("IPC listener broken pipe, shutting down");
                        break;
                    }
                    error!("IPC accept error: {}", e);
                }
            }
        }
        // Notify the handle that the server has stopped.
        let _ = tx.send(()).await;
    });

    Ok(IpcServerHandle { _receiver: rx })
}
