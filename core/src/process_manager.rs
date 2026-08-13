//! Process lifecycle management for SWAI.
//!
//! Defines a `ProcessGuard` trait with a Linux implementation, and provides
//! high-level operations: start, stop, switch, and zombie-port handling.

use nix::sys::signal::{SIGINT, SIGKILL};
use nix::unistd::getpgid;
pub use nix::unistd::Pid;
use std::net::TcpStream;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::os::unix::io::AsRawFd;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Duration;
use thiserror::Error;
use tracing::{debug, error, info, warn};

use crate::config::Config;
use crate::health_monitor::HealthMonitor;

/// Error types for process management operations.
#[derive(Debug, Error)]
pub enum ProcessError {
    #[error("another model is already running")]
    AnotherModelRunning,

    #[error("model '{0}' is not running")]
    NotRunning(String),

    #[error("port {port} occupied by unknown process (pid: {pid})")]
    PortOccupiedByUnknownProcess { pid: u32, port: u16 },

    #[error("failed to spawn process: {0}")]
    Spawn(#[from] std::io::Error),

    #[error("process exited unexpectedly with code: {0}")]
    UnexpectedExit(i32),

    #[error("shutdown timeout exceeded for model '{0}'")]
    ShutdownTimeout(String),

    #[error("port {0} still occupied after shutdown timeout")]
    PortStillOccupied(u16),

    #[error("health check failed during startup of '{0}'")]
    HealthCheckFailed(String),

    #[error("I/O error: {0}")]
    Io(std::io::Error),

    #[error("signal error: {0}")]
    Signal(#[from] nix::Error),

    #[error("cannot signal process group — target is the current process group (pid: {pid})")]
    CannotSignalOwnProcessGroup { pid: i32 },
}

impl From<config::ConfigError> for ProcessError {
    fn from(e: config::ConfigError) -> Self {
        ProcessError::Io(std::io::Error::other(
            format!("config error: {}", e),
        ))
    }
}

impl From<toml::de::Error> for ProcessError {
    fn from(e: toml::de::Error) -> Self {
        ProcessError::Io(std::io::Error::other(
            format!("toml parse error: {}", e),
        ))
    }
}

/// Guard for a running model process.
pub trait ProcessGuard: Send + Sync {
    /// Set up and start the model process.
    fn setup(script: &Path, port: u16, log_dir: &Path) -> Result<Self, ProcessError>
    where
        Self: Sized;

    /// Terminate the model process.
    /// If `fast_shutdown` is true, escalate to SIGKILL after 500ms instead of
    /// waiting up to `shutdown_timeout_sec`, releasing GPU memory instantly.
    fn terminate(&self, fast_shutdown: bool) -> Result<(), ProcessError>;
}

/// Linux implementation of ProcessGuard.
#[cfg(target_os = "linux")]
pub struct LinuxProcessGuard {
    pub pid: Option<Pid>,
    #[allow(dead_code)]
    pub port: u16,
    pub shutdown_timeout_sec: u16,
}

#[cfg(target_os = "linux")]
impl LinuxProcessGuard {
    /// Set up and start the model process on Linux using `std::process::Command`.
    ///
    /// Uses `Command::pre_exec` to set PDEATHSIG and create a new session in
    /// the child process — this is async-signal-safe because it runs after the
    /// fork but before exec, in a context where no std library locks are held.
    ///
    /// The PORT environment variable is passed via `Command::env` (not
    /// `std::env::set_var`) to avoid unsafe mutation between fork and exec.
    fn setup(script: &Path, port: u16, log_dir: &Path) -> Result<Self, ProcessError> {
        let log_file = Self::open_log_file(script, log_dir)?;

        // Rotate old log files for this model (keep most recent 20).
        let script_stem = script.file_stem().unwrap_or_default().to_string_lossy().to_string();
        Self::rotate_logs(log_dir, &script_stem, 20);

        // Keep the parent's copy of the log file fd open for future writes.
        let log_file_fd = log_file.as_raw_fd();

        // Use std::process::Command for safe fork/exec.
        // Command handles the fork internally, then runs pre_exec in the child
        // after fork but before exec — this is the safe window for PDEATHSIG.
        let mut cmd = Command::new("/bin/bash");
        cmd.arg(script)
            .env("PORT", port.to_string())
            .stdout(Stdio::from(log_file.try_clone().map_err(ProcessError::Io)?))
            .stderr(Stdio::from(log_file.try_clone().map_err(ProcessError::Io)?));

        // SAFETY: This unsafe block wraps `cmd.pre_exec()`, which runs in the child
        // process after `fork()` but before `execve()`. The only syscalls used are
        // `libc::prctl` (set PDEATHSIG), `libc::getppid()` (check parent liveness),
        // `libc::setsid()` (create new session/process group), and `libc::dup2()`
        // (redirect stdout/stderr). All of these are async-signal-safe per POSIX,
        // and no std library locks or heap allocations are performed in this context.
        // nosemgrep: rust.lang.security.unsafe-usage.unsafe-usage
        unsafe {
            cmd.pre_exec(move || {
                // NOTE: PR_SET_PDEATHSIG is intentionally NOT used here.
                // On Linux, PDEATHSIG fires when the *thread* that called
                // fork() exits, not the process. Since model scripts are
                // spawned from ephemeral background threads, the child
                // would get SIGTERM'd when the thread finishes its work.
                // Process lifecycle is managed explicitly via stop_model().

                // Create a new session so we're not a child of the parent's process group.
                let ret = libc::setsid();
                if ret < 0 {
                    return std::io::Result::Err(std::io::Error::last_os_error());
                }

                // Redirect stdout/stderr to log file (dup2 is async-signal-safe).
                let ret = libc::dup2(log_file_fd, libc::STDOUT_FILENO);
                if ret < 0 {
                    return std::io::Result::Err(std::io::Error::last_os_error());
                }
                let ret = libc::dup2(log_file_fd, libc::STDERR_FILENO);
                if ret < 0 {
                    return std::io::Result::Err(std::io::Error::last_os_error());
                }

                std::io::Result::Ok(())
            });
        }

        // Spawn the child process.
        let child = cmd.spawn().map_err(ProcessError::Spawn)?;

        let pid = Pid::from_raw(child.id() as i32);

        // Spawn a background thread to reap the child process exit status
        // when it eventually terminates, preventing <defunct> zombie processes.
        std::thread::spawn(move || {
            let mut child = child;
            let _ = child.wait();
        });

        Ok(Self {
            pid: Some(pid),
            port,
            shutdown_timeout_sec: 10,
        })
    }

    /// Get the PID of the running process.
    pub fn pid(&self) -> Option<Pid> {
        self.pid
    }

    fn open_log_file(script: &Path, log_dir: &Path) -> Result<std::fs::File, ProcessError> {
        use std::fs;
        fs::create_dir_all(log_dir).map_err(ProcessError::Io)?;

        let timestamp = chrono::Local::now().format("%Y%m%d_%H%M%S");
        let filename = format!("{}_{}.log", script.file_stem().unwrap_or_default().to_string_lossy(), timestamp);
        let log_path = log_dir.join(filename);

        // Open with restrictive permissions (0o600) so only the owner can read/write.
        // This hardens against other local users on a shared machine.
        let file = fs::OpenOptions::new()
            .create(true)
            
            .append(true)
            .mode(0o600)
            .open(&log_path)
            .map_err(ProcessError::Io)?;

        // Ensure permissions are set correctly even if umask overrides the open mode.
        fs::set_permissions(&log_path, fs::Permissions::from_mode(0o600))
            .map_err(ProcessError::Io)?;

        Ok(file)
    }

    /// Rotate (delete old) log files for a given model, keeping only the
    /// most recent `keep` files.
    ///
    /// Log files follow the pattern `{script_stem}_{YYYYMMDD_HHMMSS}.log`.
    /// This function scans the log directory, filters by script stem, and
    /// deletes files beyond the retention count (default 20).
    pub fn rotate_logs(log_dir: &Path, script_stem: &str, keep: usize) {
        let mut entries: Vec<std::fs::DirEntry> = match std::fs::read_dir(log_dir) {
            Ok(rd) => rd.filter_map(|e| e.ok()).collect(),
            Err(_) => return, // Directory doesn't exist — nothing to rotate.
        };

        // Filter to matching log files.
        entries.retain(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            name.ends_with(".log") && name.starts_with(&format!("{}_", script_stem))
        });

        // Sort descending by filename (timestamps are zero-padded).
        entries.sort_by(|a, b| {
            b.file_name()
                .cmp(&a.file_name())
        });

        // Delete excess files.
        for entry in entries.iter().skip(keep) {
            if let Err(e) = std::fs::remove_file(entry.path()) {
                warn!("failed to delete old log file {:?}: {}", entry.path(), e);
            } else {
                debug!("rotated old log file: {:?}", entry.path());
            }
        }
    }

    fn terminate_process_group(pid: Pid, timeout_sec: u16, fast_shutdown: bool) -> Result<(), ProcessError> {
        let raw_pid = pid.as_raw();

        // Safety: do not signal pid <= 0 (invalid or kernel process).
        if raw_pid <= 0 {
            return Err(ProcessError::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!("cannot signal PID {} (must be positive)", raw_pid),
            )));
        }

        // Get the process group ID of the target.
        let pgid = match getpgid(Some(pid)) {
            Ok(pgid) => pgid,
            // ESRCH: process already gone — treat as dead.
            Err(nix::errno::Errno::ESRCH) => {
                debug!("process {} already gone (ESRCH)", pid);
                return Ok(());
            }
            Err(e) => {
                return Err(ProcessError::Signal(e));
            }
        };

        // SAFETY: `libc::getpgid(0)` reads the calling thread's own process group ID.
        // It is async-signal-safe, takes no arguments that could be invalid, and only
        // reads process metadata — it never modifies state or touches the caller's PGID.
        // nosemgrep: rust.lang.security.unsafe-usage.unsafe-usage
        let our_pgid = unsafe { libc::getpgid(0) };
        if our_pgid >= 0
            && pgid.as_raw() == our_pgid {
                return Err(ProcessError::CannotSignalOwnProcessGroup { pid: raw_pid });
            }

        // Helper closure to signal the process group via -PGID.
        let signal_target = |target: Pid, sig: nix::sys::signal::Signal| -> Result<(), ProcessError> {
            let raw = target.as_raw();
            let sig_raw = sig as libc::c_int;
            // SAFETY: `libc::kill(-raw, sig_raw)` targets the process group by negating
            // the PGID per the documented POSIX convention (negative PID means "target
            // process group"). We guard against invalid PIDs above (raw <= 0 check) and
            // never signal our own process group (pgid comparison). The syscall is
            // async-signal-safe and uses only well-defined integer arguments.
            // nosemgrep: rust.lang.security.unsafe-usage.unsafe-usage
            let ret = unsafe { libc::kill(-raw, sig_raw) };
            if ret != 0 {
                let e = std::io::Error::last_os_error();
                // ESRCH: process already gone — not an error.
                if e.raw_os_error() == Some(libc::ESRCH) {
                    return Ok(());
                }
                return Err(ProcessError::Io(e));
            }
            Ok(())
        };

        // Step a: Send SIGINT to the process group (llama.cpp natively catches
        // Ctrl+C → immediately unmapping CUDA/ROCm VRAM buffers ~100ms).
        signal_target(pgid, SIGINT)?;

        // Step b: Wait/poll until the process exits or shutdown timeout expires.
        // During fast shutdown (app close), escalate to SIGKILL after 500ms to
        // release GPU memory instantly instead of waiting up to 10s.
        let graceful_timeout = if fast_shutdown {
            Duration::from_millis(500)
        } else {
            Duration::from_secs(timeout_sec as u64)
        };
        let deadline = std::time::Instant::now() + graceful_timeout;
        loop {
            match nix::sys::wait::waitpid(pid, Some(nix::sys::wait::WaitPidFlag::WNOHANG)) {
                Ok(nix::sys::wait::WaitStatus::StillAlive) => {
                    if std::time::Instant::now() >= deadline {
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(100));
                }
                Ok(_) | Err(_) => return Ok(()),
            }
        }

        // Step c: If still alive, send SIGKILL to the process group.
        warn!("process group {} didn't shut down gracefully, sending SIGKILL", pgid);
        signal_target(pgid, SIGKILL)?;

        // Step d: Wait/reap the child after SIGKILL (up to 5s).
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        loop {
            match nix::sys::wait::waitpid(pid, Some(nix::sys::wait::WaitPidFlag::WNOHANG)) {
                Ok(nix::sys::wait::WaitStatus::StillAlive) => {
                    if std::time::Instant::now() >= deadline {
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(100));
                }
                Ok(_) | Err(_) => return Ok(()),
            }
        }

        Err(ProcessError::ShutdownTimeout("unknown".to_string()))
    }

    /// Wait for the running model to become healthy by polling `/v1/models`.
    ///
    /// This method uses `HealthMonitor` to track startup progress:
    /// Starting → Loading → Ready (or Error on timeout).
    ///
    /// Returns the final state after monitoring completes or times out.
    pub fn wait_until_ready(&self) -> Result<ModelState, ProcessError> {
        let monitor = HealthMonitor::new(self.port, 30);
        monitor.wait_until_ready()
    }
}

#[cfg(target_os = "linux")]
impl ProcessGuard for LinuxProcessGuard {
    fn setup(script: &Path, port: u16, log_dir: &Path) -> Result<Self, ProcessError>
    where
        Self: Sized,
    {
        Self::setup(script, port, log_dir)
    }

    fn terminate(&self, fast_shutdown: bool) -> Result<(), ProcessError> {
        if let Some(pid) = self.pid {
            Self::terminate_process_group(pid, self.shutdown_timeout_sec, fast_shutdown)?;
        }
        Ok(())
    }
}

/// The current state of a model.
#[derive(Debug, Clone, PartialEq)]
pub enum ModelState {
    Stopped,
    Starting,
    Loading,
    Ready,
    Error(String),
}

/// A running model's metadata.
pub struct RunningModel {
    pub id: String,
    pub guard: Box<dyn ProcessGuard>,
    pub state: ModelState,
}

impl std::fmt::Debug for RunningModel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RunningModel")
            .field("id", &self.id)
            .field("state", &self.state)
            .finish()
    }
}

/// Process manager — manages the lifecycle of model processes.
///
/// Supports running multiple models concurrently (up to `max_concurrent_models`
/// from preferences). The first model started becomes the "primary" active
/// model; subsequent models are added to the running set.
pub struct ProcessManager {
    config: Config,
    running_models: Vec<RunningModel>,
    /// Index of the primary (first-started) model in `running_models`.
    /// `None` when no models are running.
    primary_index: Option<usize>,
}

impl ProcessManager {
    /// Create a new process manager from a loaded config.
    pub fn new(config: Config) -> Self {
        Self {
            config,
            running_models: Vec::new(),
            primary_index: None,
        }
    }

    /// Access the underlying config for reconciliation at startup.
    pub fn config(&self) -> &Config {
        &self.config
    }

    /// Maximum number of concurrent models allowed (from preferences).
    pub fn max_concurrent_models(&self) -> usize {
        self.config.max_concurrent_models()
    }

    /// Update the in-memory config (e.g. after saving preferences).
    pub fn update_config(&mut self, new_config: Config) {
        self.config = new_config;
    }

    /// Number of currently running models.
    pub fn running_count(&self) -> usize {
        self.running_models.len()
    }

    /// Add a newly imported model to the in-memory config.
    pub fn add_model(&mut self, model: crate::config::ModelConfig) {
        if !self.config.models.iter().any(|m| m.id == model.id) {
            self.config.models.push(model);
        }
    }

    /// Remove a model by id from the in-memory config.
    ///
    /// If the model is currently running, stops it first (graceful shutdown).
    /// Returns `Err` if the model is not found in config.
    pub fn remove_model(&mut self, id: &str) -> Result<(), String> {
        // If the model is currently running, stop it first.
        if self.running_models.iter().position(|m| m.id == id).is_some() {
            info!("stopping model '{}' before removal", id);
            // Use graceful shutdown (not fast) — deletion isn't an emergency.
            if let Err(e) = self.stop_model(id, false) {
                warn!("failed to stop model '{}' before removal: {}", id, e);
            }
        }

        // Remove from config.models.
        let initial_len = self.config.models.len();
        self.config.models.retain(|m| m.id != id);

        if self.config.models.len() == initial_len {
            return Err(format!("model '{}' not found in config", id));
        }

        info!("removed model '{}' from config", id);
        Ok(())
    }

    /// Start a model by id. Returns `Err` if the concurrent model limit is reached.
    pub fn start_model(&mut self, id: &str) -> Result<(), ProcessError> {
        // Check concurrent model limit
        let max = self.max_concurrent_models();
        if self.running_models.len() >= max {
            return Err(ProcessError::AnotherModelRunning);
        }

        // Find the model config
        let model_config = self
            .config
            .models
            .iter()
            .find(|m| m.id == id)
            .ok_or_else(|| ProcessError::NotRunning(id.to_string()))?;

        // Check port — zombie port handling
        match Self::check_port(model_config.port) {
            PortState::Free => {}
            PortState::OccupiedByModel => {
                return Err(ProcessError::HealthCheckFailed(format!(
                    "model '{}' is already running on port {}",
                    id, model_config.port
                )));
            }
            PortState::OccupiedByUnknown(pid) => {
                return Err(ProcessError::PortOccupiedByUnknownProcess {
                    pid,
                    port: model_config.port,
                });
            }
        }

        // Spawn the process (synchronously before entering async context)
        let log_dir = self.config.log_dir();
        let guard_result = LinuxProcessGuard::setup(
            &model_config.script_path,
            model_config.port,
            &log_dir,
        );

        match guard_result {
            Ok(guard) => {
                info!("started model '{}' on port {}", id, model_config.port);
                let idx = self.running_models.len();
                let is_primary = self.primary_index.is_none();
                self.running_models.push(RunningModel {
                    id: id.to_string(),
                    guard: Box::new(guard),
                    state: ModelState::Starting,
                });
                if is_primary {
                    self.primary_index = Some(idx);
                }
                Ok(())
            }
            Err(e) => {
                error!("failed to start model '{}': {}", id, e);
                Err(e)
            }
        }
    }

    /// Stop a model by id.
    /// If `fast_shutdown` is true, escalate to SIGKILL after 500ms instead of
    /// waiting up to `shutdown_timeout_sec`, releasing GPU memory instantly.
    pub fn stop_model(&mut self, id: &str, fast_shutdown: bool) -> Result<(), ProcessError> {
        let idx = self
            .running_models
            .iter()
            .position(|m| m.id == id)
            .ok_or_else(|| ProcessError::NotRunning(id.to_string()))?;

        // Get the port from the config for the running model
        let port = self
            .config
            .models
            .iter()
            .find(|m| m.id == id)
            .map(|m| m.port)
            .ok_or_else(|| ProcessError::NotRunning(id.to_string()))?;

        // Terminate the process group
        self.running_models.remove(idx).guard.terminate(fast_shutdown)?;

        // Confirm port is free via TCP bind retry (up to 10s)
        Self::wait_port_free(port, Duration::from_secs(10))?;

        // Adjust primary_index if needed
        if self.primary_index == Some(idx) {
            self.primary_index = None;
        } else if let Some(ref mut pi) = self.primary_index {
            if *pi > idx {
                *pi -= 1;
            }
        }

        info!("stopped model '{}'", id);
        Ok(())
    }

    /// Stop all running models.
    pub fn stop_all(&mut self, fast_shutdown: bool) -> Result<(), ProcessError> {
        // Stop in reverse order so primary_index adjustments stay valid.
        while let Some(running) = self.running_models.pop() {
            let port = self
                .config
                .models
                .iter()
                .find(|m| m.id == running.id)
                .map(|m| m.port)
                .ok_or_else(|| ProcessError::NotRunning(running.id.clone()))?;
            running.guard.terminate(fast_shutdown)?;
            Self::wait_port_free(port, Duration::from_secs(10))?;
        }
        self.primary_index = None;
        Ok(())
    }

    /// Switch from one model to another (atomic sequence).
    ///
    /// Stops the current model, waits 500ms for CUDA context release,
    /// then starts the new model. If any step fails, affected state is cleaned up.
    pub fn switch_model(&mut self, from_id: &str, to_id: &str) -> Result<(), ProcessError> {
        // Step 1: stop the current model
        if let Err(e) = self.stop_model(from_id, false) {
            return Err(e);
        }

        // Step 2: short delay for CUDA context release
        std::thread::sleep(Duration::from_millis(500));

        // Step 3: start the new model
        match self.start_model(to_id) {
            Ok(()) => Ok(()),
            Err(e) => Err(e),
        }
    }

    /// Get the currently running models.
    pub fn get_running_models(&self) -> &[RunningModel] {
        &self.running_models
    }

    /// Get the primary (first-started) running model, if any.
    pub fn get_primary_model(&self) -> Option<&RunningModel> {
        self.primary_index
            .and_then(|i| self.running_models.get(i))
    }

    /// Get the primary running model id, if any.
    pub fn get_primary_model_id(&self) -> Option<&str> {
        self.primary_index
            .and_then(|i| self.running_models.get(i).map(|m| m.id.as_str()))
    }

    /// Find a running model by id.
    pub fn find_running_model(&self, id: &str) -> Option<&RunningModel> {
        self.running_models.iter().find(|m| m.id == id)
    }

    /// Get all running model IDs.
    pub fn running_model_ids(&self) -> Vec<String> {
        self.running_models.iter().map(|m| m.id.clone()).collect()
    }

    /// Build a map of all running model IDs to their configured ports.
    ///
    /// Used by the proxy to register all concurrently running models so that
    /// dynamic routing can dispatch requests to the correct model based on the
    /// `model` field in incoming requests.
    pub fn running_model_ports(&self) -> Vec<(String, u16)> {
        self.running_models
            .iter()
            .filter_map(|m| {
                self.config
                    .models
                    .iter()
                    .find(|c| c.id == m.id)
                    .map(|c| (m.id.clone(), c.port))
            })
            .collect()
    }

    /// Get the port for a running model by id.
    pub fn get_port_for_model(&self, id: &str) -> Option<u16> {
        self.config
            .models
            .iter()
            .find(|m| m.id == id)
            .map(|m| m.port)
    }

    /// Set a running model (used during reconciliation at startup).
    pub fn set_running_model(&mut self, model: RunningModel) {
        let idx = self.running_models.len();
        if self.primary_index.is_none() {
            self.primary_index = Some(idx);
        }
        self.running_models.push(model);
    }

    /// Check if a port is free or occupied by a llama-server process.
    pub fn check_port(port: u16) -> PortState {
        // Try to bind the port
        if TcpStream::connect(format!("127.0.0.1:{}", port)).is_ok() {
            // Port is occupied — probe it for /v1/models with a short timeout
            // to prevent indefinite hangs on unresponsive listeners.
            let url = format!("http://127.0.0.1:{}/v1/models", port);
            if let Ok(client) = reqwest::blocking::Client::builder()
                .timeout(Duration::from_secs(2))
                .build()
            {
                if let Ok(resp) = client.get(&url).send() {
                    if resp.status().is_success() {
                        // Check if it's a llama-server response (has /v1/models endpoint)
                        if let Ok(body) = resp.text() {
                            if body.contains("\"id\"") && body.contains("model") {
                                return PortState::OccupiedByModel;
                            }
                        }
                    }
                }
            }
            // Occupied but not a llama-server — try to get pid
            if let Ok(pid) = Self::get_port_pid(port) {
                return PortState::OccupiedByUnknown(pid);
            }
            PortState::OccupiedByUnknown(0)
        } else {
            PortState::Free
        }
    }

    /// Wait for a port to become free.
    fn wait_port_free(port: u16, timeout: Duration) -> Result<(), ProcessError> {
        let deadline = std::time::Instant::now() + timeout;
        loop {
            if TcpStream::connect(format!("127.0.0.1:{}", port)).is_err() {
                return Ok(()); // port is free
            }
            if std::time::Instant::now() >= deadline {
                return Err(ProcessError::PortStillOccupied(port));
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    /// Get the PID of a process bound to a port (best effort).
    ///
    /// Parses `/proc/net/tcp` to find the inode for the given port, then
    /// searches `/proc/*/fd` for that inode to identify the owning PID.
    pub fn get_port_pid(port: u16) -> Result<u32, ProcessError> {
        // Use /proc/net/tcp to find the PID — this is Linux-specific
        #[cfg(target_os = "linux")]
        {
            let tcp_path = "/proc/net/tcp";
            if let Ok(content) = std::fs::read_to_string(tcp_path) {
                for line in content.lines().skip(1) {
                    // Format: sl local_address remote_address ... state ... inode ...
                    // Fields:  0 1              2          ... 7      ... 9
                    // parts[9] is the inode — need >= 10 fields total.
                    let parts: Vec<&str> = line.split_whitespace().collect();
                    if parts.len() >= 10 {
                        let local_addr = parts[1];
                        let local_port = u16::from_str_radix(local_addr.split(':').nth(1).unwrap_or_default(), 16).ok();
                        if local_port == Some(port) {
                            // Parse inode to find the PID
                            let inode = parts[9];
                            if let Ok(inode_num) = inode.parse::<u32>() {
                                // Search /proc/*/fd for this inode
                                if let Ok(entries) = std::fs::read_dir("/proc") {
                                    for entry in entries.flatten() {
                                        let fd_path = entry.path().join("fd");
                                        if let Ok(fds) = std::fs::read_dir(&fd_path) {
                                            for fd_entry in fds.flatten() {
                                                if let Ok(link) = std::fs::read_link(fd_entry.path()) {
                                                    if link.to_string_lossy().contains(&inode_num.to_string()) {
                                                        // entry.file_name() returns the bare numeric directory name
                                                        // (e.g. "1234"), not a path with a prefix.
                                                        if let Ok(pid) = entry.file_name().to_string_lossy().parse::<u32>() {
                                                            return Ok(pid);
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        Err(ProcessError::Io(std::io::Error::other(
            "couldn't determine port PID",
        )))
    }

    /// Start a model and report intermediate health states through a channel.
    ///
    /// This is the full startup flow for the UI:
    /// 1. Call `start_model()` (process spawns, state = Starting)
    /// 2. Spawn a background thread that polls `/v1/models` via `HealthMonitor`
    /// 3. Send `StateUpdate(Starting)` → `StateUpdate(Loading)` → ... → `StateUpdate(Ready|Error)`
    /// 4. Send `SwitchCompleted` with the final result
    ///
    /// The caller should spawn this on a background thread and not await it.
    pub fn start_model_and_report(
        &mut self,
        id: &str,
        tx: std::sync::mpsc::Sender<ModelState>,
    ) -> Result<(), ProcessError> {
        // Start the model (spawns process, sets state = Starting)
        self.start_model(id)?;

        // Extract the port from config for health monitoring
        let model_port = self.config.models.iter()
            .find(|m| m.id == id)
            .map(|m| m.port);

        if let Some(port) = model_port {
            // Spawn background health monitor thread that reports state changes.
            let monitor = HealthMonitor::new(port, 30);
            std::thread::spawn(move || {
                monitor.wait_until_ready_with_updates(tx);
            });
        }

        Ok(())
    }

    /// Resolve a model identifier (id or name) to the port of its running instance.
    ///
    /// Returns `None` if the model is not currently running, allowing the proxy
    /// to fall back to the primary active model.
    pub fn resolve_running_port(&self, identifier: &str) -> Option<u16> {
        // Try matching by id first, then by name (config.name).
        for model in &self.running_models {
            if model.id == identifier {
                return self.get_port_for_model(&model.id);
            }
        }
        // Check config names for a running model.
        if self.config.models.iter().any(|c| c.name == identifier) {
            // Find the running model that matches this name.
            for model in &self.running_models {
                if let Some(cfg) = self.config.models.iter().find(|c| c.name == identifier && c.id == model.id) {
                    return Some(cfg.port);
                }
            }
        }
        None
    }
}

/// State of a network port.
#[derive(Debug, PartialEq)]
pub enum PortState {
    Free,
    OccupiedByModel,
    OccupiedByUnknown(u32),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Config, GlobalSettings, PreferencesConfig};

    #[test]
    fn test_port_state_free() {
        // Port 9999 should be free (unless something is running on it)
        let state = ProcessManager::check_port(9999);
        assert_eq!(state, PortState::Free);
    }

    #[test]
    fn test_process_error_variants() {
        let err = ProcessError::AnotherModelRunning;
        assert!(err.to_string().contains("another model"));

        let err = ProcessError::NotRunning("m1".to_string());
        assert!(err.to_string().contains("m1"));

        let err = ProcessError::PortOccupiedByUnknownProcess { pid: 1234, port: 8081 };
        assert!(err.to_string().contains("8081"));
        assert!(err.to_string().contains("1234"));
    }

    #[test]
    fn test_process_manager_multi_model_state() {
        // Verify the ProcessManager struct supports multiple running models.
        let _tmp = tempfile::tempdir().unwrap();
        let config = Config {
            schema_version: 1,
            models: vec![],
            global: GlobalSettings::default(),
            preferences: PreferencesConfig {
                auto_follow_logs: true,
                enable_notifications: true,
                notify_on_switch: true,
                autostart_on_login: false,
                max_concurrent_models: 4,
            },
        };
        let pm = ProcessManager::new(config);
        assert_eq!(pm.running_count(), 0);
        assert!(pm.get_primary_model().is_none());
        assert!(pm.get_primary_model_id().is_none());
    }
}
