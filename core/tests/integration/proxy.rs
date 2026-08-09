use swai_core::proxy::*;
use swai_core::config::Config;
use std::sync::Arc;
use tokio::net::TcpListener;

#[tokio::test]
async fn proxy_creates_and_starts() {
    let cfg = Config::default();
    let proxy = Proxy::new(cfg);
    assert_eq!(proxy.state().read().unwrap().bound, false);
}

#[tokio::test]
async fn proxy_bind_and_shutdown() {
    let cfg = Config::default();
    let proxy = Arc::new(Proxy::new(cfg));

    // Use a random available port via 0.0.0.0:0
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let proxy_clone = Arc::clone(&proxy);
    let handle = tokio::spawn(async move {
        proxy_clone.start(listener).await;
    });

    // Give it a moment to start
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
    assert!(proxy.state().read().unwrap().bound);

    // Shutdown
    proxy.shutdown();
    handle.await.unwrap();
}

#[test]
fn proxy_state_update() {
    let cfg = Config::default();
    let proxy = Proxy::new(cfg);
    let mut state = proxy.state().write().unwrap();
    state.bound = true;
    drop(state);
    assert!(proxy.state().read().unwrap().bound);
}
