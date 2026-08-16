//! Integration tests for SWAI — Phase 3 process manager resilience.
//!
//! Tests the repeated switch loop and zombie-port recovery paths using real
//! model scripts and real subprocesses. No orphan processes should be left
//! behind after these tests complete.

use serial_test::serial;
use std::net::TcpStream;
use std::process::Child;
use std::thread;
use std::time::{Duration, Instant};

use swai_core::config::Config;
use swai_core::process_manager::{ProcessError, ProcessManager};
use swai_core::run;

/// Path to the integration test config.
const INTEGRATION_CONFIG: &str =
    "/mnt/orico/Documents/ApplicationsRAW/swai/core/tests/integration_tests.toml";

/// Test ports used by model-a and model-b.
const TEST_PORT_A: u16 = 9876;
const TEST_PORT_B: u16 = 9877;

/// Load config from the integration test path. Returns the config and a guard
/// that keeps the single-instance lock alive for the duration of the test.
fn load_test_config() -> (Config, swai_core::single_instance::SingleInstanceGuard) {
    let (config, _reconcile_result, guard) =
        run(Some(INTEGRATION_CONFIG)).expect("should load integration config");
    (config, guard)
}

/// Check if a port is bound (i.e., we can connect to it).
fn is_port_bound(port: u16) -> bool {
    TcpStream::connect(format!("127.0.0.1:{}", port)).is_ok()
}

/// Wait for a port to become bound (up to timeout).
fn wait_port_bound(port: u16, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if is_port_bound(port) {
            return true;
        }
        thread::sleep(Duration::from_millis(50));
    }
    is_port_bound(port)
}

/// Wait for a port to become free (up to timeout).
fn wait_port_free(port: u16, timeout: Duration) -> Result<(), ProcessError> {
    let deadline = Instant::now() + timeout;
    loop {
        if !is_port_bound(port) {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(ProcessError::PortStillOccupied(port));
        }
        thread::sleep(Duration::from_millis(100));
    }
}

/// Drop guard that ensures a nc zombie listener is cleaned up even on panic.
struct ZombieListenerGuard {
    child: Option<Child>,
    #[allow(dead_code)]
    port: u16,
}

impl ZombieListenerGuard {
    fn new(child: Child, port: u16) -> Self {
        Self {
            child: Some(child),
            port,
        }
    }
}

impl Drop for ZombieListenerGuard {
    fn drop(&mut self) {
        if let Some(ref mut child) = self.child {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

/// Verify that the test ports are free (no orphan processes).
fn verify_ports_free() {
    let _ = wait_port_free(TEST_PORT_A, Duration::from_secs(2));
    let _ = wait_port_free(TEST_PORT_B, Duration::from_secs(2));
    if is_port_bound(TEST_PORT_A) {
        panic!(
            "Test port {} is already occupied. Inspect with:\n\
             ss -tlnp | grep {}\n\
             and kill only the exact leftover PID, or use:\n\
             fuser -k {}/tcp",
            TEST_PORT_A, TEST_PORT_A, TEST_PORT_A
        );
    }
    if is_port_bound(TEST_PORT_B) {
        panic!(
            "Test port {} is already occupied. Inspect with:\n\
             ss -tlnp | grep {}\n\
             and kill only the exact leftover PID, or use:\n\
             fuser -k {}/tcp",
            TEST_PORT_B, TEST_PORT_B, TEST_PORT_B
        );
    }
}

// ─── Repeated switch loop test ────────────────────────────────────────────────

#[serial]
#[test]
fn test_repeated_switch_loop() {
    verify_ports_free();

    let (config, _guard) = load_test_config();
    let mut pm = ProcessManager::new(config);

    // Initial start: model-a.
    pm.start_model("model-a")
        .expect("initial start model-a failed");
    assert!(
        wait_port_bound(TEST_PORT_A, Duration::from_secs(3)),
        "port {} should be bound after starting model-a",
        TEST_PORT_A
    );

    for i in 0..10 {
        let (from, to, expected_port) = if i % 2 == 0 {
            ("model-a", "model-b", TEST_PORT_B)
        } else {
            ("model-b", "model-a", TEST_PORT_A)
        };

        let result = pm.switch_model(from, to);
        assert!(
            result.is_ok(),
            "switch {} → {} failed on iteration {}: {:?}",
            from,
            to,
            i,
            result
        );

        assert!(
            wait_port_bound(expected_port, Duration::from_secs(3)),
            "port {} should be bound after switching to {} on iteration {}",
            expected_port,
            to,
            i
        );
    }

    // Stop the last running model (model-a after 10 switches: A→B→A→B→A→B→A→B→A→B→A).
    pm.stop_model("model-a", false)
        .expect("should stop model-a");

    verify_ports_free();
    println!("Repeated switch loop passed: 10 iterations without errors or port conflicts");
}

// ─── Zombie port recovery test ────────────────────────────────────────────────

#[serial]
#[test]
fn test_zombie_port_path() {
    verify_ports_free();

    let (config, _guard) = load_test_config();
    let mut pm = ProcessManager::new(config);

    let zombie = std::process::Command::new("nc")
        .args(["-l", "-k", "9876"])
        .spawn()
        .expect("should spawn nc listener to simulate zombie port");

    let _zombie_guard = ZombieListenerGuard::new(zombie, TEST_PORT_A);
    thread::sleep(Duration::from_millis(500));

    assert!(
        is_port_bound(TEST_PORT_A),
        "zombie nc process should occupy port 9876"
    );

    let start_result = pm.start_model("model-a");
    assert!(
        start_result.is_err(),
        "start_model should fail when port is occupied by foreign process"
    );
    match start_result.unwrap_err() {
        ProcessError::PortOccupiedByUnknownProcess { port, .. } => {
            assert_eq!(port, TEST_PORT_A, "expected port {} in error", TEST_PORT_A);
        }
        other => panic!("expected PortAlreadyBound, got: {:?}", other),
    }

    let fake_running = swai_core::process_manager::RunningModel {
        id: "fake-model".to_string(),
        guard: Box::new(swai_core::process_manager::LinuxProcessGuard {
            pid: None,
            port: 9877,
            shutdown_timeout_sec: 10,
        }),
        state: swai_core::process_manager::ModelState::Ready,
    };
    pm.set_running_model(fake_running);

    let switch_result = pm.switch_model("fake-model", "model-a");
    assert!(
        switch_result.is_err(),
        "switch_model should fail before touching running model when target port is occupied"
    );

    assert_eq!(
        pm.get_primary_model_id(),
        Some("fake-model"),
        "fake-model should still be primary since switch failed pre-flight"
    );

    drop(_zombie_guard);
    let _ = wait_port_free(TEST_PORT_A, Duration::from_secs(2));

    verify_ports_free();
    println!("Zombie port recovery test passed: alien processes prevented start/switch cleanly");
}

// ─── Port free check during switch ────────────────────────────────────────────

#[serial]
#[test]
fn test_port_free_between_switch_steps() {
    verify_ports_free();

    let (config, _guard) = load_test_config();
    let mut pm = ProcessManager::new(config);

    pm.start_model("model-a").expect("should start model-a");
    assert!(
        wait_port_bound(TEST_PORT_A, Duration::from_secs(3)),
        "port {} should be bound after starting model-a",
        TEST_PORT_A
    );

    let result = pm.switch_model("model-a", "model-b");
    assert!(result.is_ok(), "switch a→b should succeed");

    assert!(
        !is_port_bound(TEST_PORT_A),
        "port {} should be free after switching away from model-a",
        TEST_PORT_A
    );

    assert!(
        wait_port_bound(TEST_PORT_B, Duration::from_secs(3)),
        "port {} should be bound after starting model-b",
        TEST_PORT_B
    );

    pm.stop_model("model-b", false)
        .expect("should stop model-b");

    verify_ports_free();
    println!("Port free check between switch steps passed");
}

// ─── Cleanup on test teardown ────────────────────────────────────────────────

#[serial]
#[test]
fn test_no_orphans_after_tests() {
    verify_ports_free();
}
