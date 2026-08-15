use std::io;
use std::os::unix::net::UnixStream;

use super::protocol::{ActionRequest, ActionResponse};
use super::server::socket_path;

#[derive(Debug)]
pub enum IpcClientError {
    /// The socket file does not exist — SWAI is not running.
    SocketNotFound,
    /// Connection refused — the server is not accepting connections.
    ConnectionRefused,
    /// An I/O error occurred during communication.
    Io(io::Error),
    /// The server returned an error response.
    ServerError(String),
}

impl std::fmt::Display for IpcClientError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            IpcClientError::SocketNotFound => {
                write!(f, "IPC socket not found — is SWAI running?")
            }
            IpcClientError::ConnectionRefused => {
                write!(f, "connection refused — is SWAI running?")
            }
            IpcClientError::Io(e) => write!(f, "I/O error: {}", e),
            IpcClientError::ServerError(msg) => write!(f, "server error: {}", msg),
        }
    }
}

impl std::error::Error for IpcClientError {}

impl From<io::Error> for IpcClientError {
    fn from(e: io::Error) -> Self {
        if e.kind() == io::ErrorKind::NotFound {
            IpcClientError::SocketNotFound
        } else if e.kind() == io::ErrorKind::ConnectionRefused {
            IpcClientError::ConnectionRefused
        } else {
            IpcClientError::Io(e)
        }
    }
}

/// Connect to the IPC server and send an action request.
///
/// Returns the parsed `ActionResponse`, or an `IpcClientError` if the
/// connection fails or the server returns an error.
pub fn ipc_send(request: &ActionRequest) -> Result<ActionResponse, IpcClientError> {
    let path = socket_path();

    // Check that the socket file exists first for a friendlier error message.
    if !path.exists() {
        return Err(IpcClientError::SocketNotFound);
    }

    // Connect to the Unix domain socket.
    let stream = UnixStream::connect(&path)?;
    stream.set_read_timeout(Some(std::time::Duration::from_secs(10)))?;

    // Serialize and send the request.
    let body = serde_json::to_string(request).map_err(|e| {
        IpcClientError::Io(io::Error::other(
            format!("request serialization error: {}", e),
        ))
    })?;

    use std::io::Write;
    let mut writer = io::BufWriter::new(&stream);
    writer.write_all(body.as_bytes())?;
    writer.write_all(b"\n")?;
    writer.flush()?;

    // Read the response.
    use std::io::BufRead;
    let mut reader = io::BufReader::new(&stream);
    let mut response_buf = String::new();
    reader.read_line(&mut response_buf)?;

    // Parse the JSON response.
    let response: ActionResponse = serde_json::from_str(&response_buf).map_err(|e| {
        IpcClientError::Io(io::Error::other(
            format!("response parse error: {}", e),
        ))
    })?;

    if response.status == "error" {
        return Err(IpcClientError::ServerError(response.message));
    }

    Ok(response)
}

// ---------------------------------------------------------------------------
