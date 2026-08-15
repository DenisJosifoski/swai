use crate::health_monitor::HealthMonitor;
use super::types::ModelState;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::os::unix::io::AsRawFd;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Duration;
use nix::sys::signal::{SIGINT, SIGKILL};
use nix::unistd::{getpgid, Pid};
use tracing::{debug, warn};

use super::error::ProcessError;


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

