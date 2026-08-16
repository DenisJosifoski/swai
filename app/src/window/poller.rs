use gio::prelude::ApplicationExt;
use gio::Notification;
use glib::object::Cast;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use swai_core::process_manager::ProcessManager;
use swai_core::proxy::ProxyState;

use super::types::{ChannelMessage, SlotInfo, SlotUpdate};

pub fn spawn_context_poller(
    pm: Arc<Mutex<ProcessManager>>,
    sender: std::sync::mpsc::Sender<ChannelMessage>,
    slot_sender: std::sync::mpsc::Sender<SlotUpdate>,
    auto_restart_enabled: bool,
    proxy_state: Option<Arc<Mutex<ProxyState>>>,
    enable_notifications: bool,
) {
    let http_client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(3))
        .build()
        .expect("failed to build reqwest blocking client");

    let poll_running = Arc::new(AtomicBool::new(true));
    let poll_running_clone = Arc::clone(&poll_running);

    std::thread::spawn(move || {
        let mut last_poll_attempt = std::time::Instant::now();
        let mut last_tokens: std::collections::HashMap<String, (usize, std::time::Instant)> =
            std::collections::HashMap::new();

        while poll_running_clone.load(Ordering::SeqCst) {
            for _ in 0..20 {
                if !poll_running_clone.load(Ordering::SeqCst) {
                    return;
                }
                std::thread::sleep(std::time::Duration::from_millis(100));
            }

            if last_poll_attempt.elapsed() < std::time::Duration::from_secs(1) {
                continue;
            }

            let ready_ports: Vec<(String, u16)> = match pm.lock() {
                Ok(pm_lock) => pm_lock.running_model_ports(),
                Err(_) => continue,
            };

            for (model_id, port) in &ready_ports {
                let url = format!("http://127.0.0.1:{}/slots", port);

                match http_client.get(&url).send() {
                    Ok(resp) => {
                        if resp.status().is_success() {
                            if let Ok(body) = resp.text() {
                                if let Some(slot_info) = parse_slots_response(&body) {
                                    let now = std::time::Instant::now();
                                    let mut calculated_speed = slot_info.predicted_per_second;

                                    // Fallback: Delta Token Speedometer if llama-server doesn't provide predicted_per_second
                                    if calculated_speed == 0.0 {
                                        if let Some(&(prev_tokens, prev_time)) =
                                            last_tokens.get(model_id)
                                        {
                                            let dt = now.duration_since(prev_time).as_secs_f64();
                                            if dt > 0.3 && slot_info.tokens_used >= prev_tokens {
                                                let delta_tokens =
                                                    slot_info.tokens_used - prev_tokens;
                                                if delta_tokens > 0 {
                                                    calculated_speed = delta_tokens as f64 / dt;
                                                }
                                            }
                                        }
                                    }
                                    last_tokens
                                        .insert(model_id.clone(), (slot_info.tokens_used, now));

                                    let _ = slot_sender.send(SlotUpdate {
                                        model_id: model_id.clone(),
                                        tokens_used: slot_info.tokens_used,
                                        n_ctx: slot_info.n_ctx,
                                        predicted_per_second: calculated_speed,
                                        prompt_per_second: slot_info.prompt_per_second,
                                    });

                                    if auto_restart_enabled
                                        && slot_info.n_ctx > 0
                                        && (slot_info.tokens_used as f64 / slot_info.n_ctx as f64)
                                            >= 0.98
                                    {
                                        trigger_auto_restart(
                                            &pm,
                                            &sender,
                                            &slot_sender,
                                            &proxy_state,
                                            model_id,
                                            enable_notifications,
                                            slot_info.n_ctx,
                                        );
                                    }
                                }
                            }
                        }
                    }
                    Err(e) => {
                        tracing::debug!(
                            "slots poll failed for model '{}' on port {}: {}",
                            model_id,
                            port,
                            e
                        );
                    }
                }
            }

            last_poll_attempt = std::time::Instant::now();
        }

        tracing::info!("context poller thread exited");
    });
}

/// Parse the /slots JSON response to extract context and speed information.
pub fn parse_slots_response(body: &str) -> Option<SlotInfo> {
    let json: serde_json::Value = serde_json::from_str(body).ok()?;

    // Helper to extract speed metrics from a slot (checks top-level, timings, metrics, and stats).
    let extract_speed = |slot: &serde_json::Value| -> (f64, f64) {
        let find_num = |key: &str| -> Option<f64> {
            slot.get(key)
                .or_else(|| slot.get("timings").and_then(|t| t.get(key)))
                .or_else(|| slot.get("metrics").and_then(|m| m.get(key)))
                .or_else(|| slot.get("stats").and_then(|s| s.get(key)))
                .and_then(|v| v.as_f64())
        };

        let predicted = find_num("predicted_per_second")
            .or_else(|| find_num("predicted_ms"))
            .or_else(|| find_num("tokens_per_second"))
            .unwrap_or(0.0);

        let prompt = find_num("prompt_per_second")
            .or_else(|| find_num("prompt_ms"))
            .unwrap_or(0.0);

        (predicted, prompt)
    };

    // Case A: Top-level Array.
    if let Some(arr) = json.as_array() {
        if let Some(first_slot) = arr.first() {
            let n_ctx = first_slot
                .get("n_ctx")
                .and_then(|v| v.as_u64())
                .unwrap_or(0) as usize;
            let mut tokens_used: u64 = 0;
            let (mut predicted_per_second, mut prompt_per_second) = (0.0, 0.0);

            for slot in arr {
                let prompt = slot
                    .get("n_prompt_tokens")
                    .or_else(|| slot.get("prompt_tokens_total"))
                    .or_else(|| slot.get("n_past"))
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);

                let gen = slot
                    .get("next_token")
                    .and_then(|v| v.as_array())
                    .and_then(|arr| arr.first())
                    .and_then(|tok| tok.get("n_decoded"))
                    .or_else(|| slot.get("generation_tokens_total"))
                    .or_else(|| slot.get("n_decoded"))
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);

                tokens_used += prompt + gen;

                // Accumulate speed metrics (take the first valid values).
                if predicted_per_second == 0.0 {
                    let (p, pr) = extract_speed(slot);
                    if p > 0.0 || pr > 0.0 {
                        predicted_per_second = p;
                        prompt_per_second = pr;
                    }
                }
            }

            if n_ctx > 0 {
                return Some(SlotInfo {
                    tokens_used: tokens_used as usize,
                    n_ctx,
                    predicted_per_second,
                    prompt_per_second,
                });
            }
        }
    }

    // Case B: Top-level Object.
    let n_ctx = json.get("n_ctx").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
    let mut tokens_used: u64 = 0;
    let (mut predicted_per_second, mut prompt_per_second) = (0.0, 0.0);

    if let Some(slots) = json.get("slots").and_then(|v| v.as_array()) {
        for slot in slots {
            let prompt = slot
                .get("n_prompt_tokens")
                .or_else(|| slot.get("prompt_tokens_total"))
                .or_else(|| slot.get("n_past"))
                .and_then(|v| v.as_u64())
                .unwrap_or(0);

            let gen = slot
                .get("next_token")
                .and_then(|v| v.as_array())
                .and_then(|arr| arr.first())
                .and_then(|tok| tok.get("n_decoded"))
                .or_else(|| slot.get("generation_tokens_total"))
                .or_else(|| slot.get("n_decoded"))
                .and_then(|v| v.as_u64())
                .unwrap_or(0);

            tokens_used += prompt + gen;

            // Accumulate speed metrics (take the first valid values).
            if predicted_per_second == 0.0 {
                let (p, pr) = extract_speed(slot);
                if p > 0.0 || pr > 0.0 {
                    predicted_per_second = p;
                    prompt_per_second = pr;
                }
            }
        }
    }

    if n_ctx > 0 {
        Some(SlotInfo {
            tokens_used: tokens_used as usize,
            n_ctx,
            predicted_per_second,
            prompt_per_second,
        })
    } else {
        None
    }
}

/// Trigger an auto-restart when context is full (>=98% of n_ctx).
pub fn trigger_auto_restart(
    pm: &Arc<Mutex<ProcessManager>>,
    sender: &std::sync::mpsc::Sender<ChannelMessage>,
    slot_sender: &std::sync::mpsc::Sender<SlotUpdate>,
    proxy_state: &Option<Arc<Mutex<ProxyState>>>,
    model_id: &str,
    enable_notifications: bool,
    n_ctx: usize,
) {
    static LAST_RESTART: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

    let now_sec = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    let last_sec = LAST_RESTART.load(Ordering::Relaxed);
    if now_sec > 0 && last_sec > 0 && now_sec < last_sec + 15 {
        tracing::info!(
            "auto-restart for '{}' suppressed by 15s cooldown guard",
            model_id
        );
        return;
    }
    LAST_RESTART.store(now_sec, Ordering::Relaxed);

    // Immediately reset the UI progress bar tokens to 0 so stale 98% metrics don't re-trigger.
    let _ = slot_sender.send(SlotUpdate {
        model_id: model_id.to_string(),
        tokens_used: 0,
        n_ctx,
        predicted_per_second: 0.0,
        prompt_per_second: 0.0,
    });

    let bg_model_id = model_id.to_string();
    let bg_pm = Arc::clone(pm);
    let bg_sender = sender.clone();
    let bg_proxy = proxy_state.clone();
    let bg_enable_notifications = enable_notifications;

    std::thread::spawn(move || {
        let _ = bg_sender.send(ChannelMessage::RestartRequested {
            model_id: bg_model_id.clone(),
        });

        let result = {
            let mut pm_lock = match bg_pm.lock() {
                Ok(g) => g,
                Err(_) => return,
            };

            if pm_lock.get_primary_model_id() == Some(bg_model_id.as_str()) {
                let _ = pm_lock.stop_model(&bg_model_id, true);
                std::thread::sleep(std::time::Duration::from_millis(1500));
            }

            pm_lock.start_model(&bg_model_id)
        };

        let is_ok = result.is_ok();

        // The health monitor drives the final state.
        // Only send SwitchCompleted on failure.
        if !is_ok {
            let _ = bg_sender.send(ChannelMessage::SwitchCompleted {
                target_id: bg_model_id.clone(),
                result,
            });
        }

        if is_ok && bg_enable_notifications {
            // Schedule notification on the main (GLib) thread.
            let auto_restart_body = format!("Context full - {} restarted", bg_model_id);
            glib::idle_add_once(move || {
                notify("SWAI", &auto_restart_body);
            });

            if let Some(ref proxy) = bg_proxy {
                let mut ps = proxy.lock().unwrap_or_else(|e| {
                    tracing::error!("auto-restart: proxy state lock poisoned");
                    e.into_inner()
                });
                let running = bg_pm
                    .lock()
                    .ok()
                    .map(|pm| pm.running_model_ports())
                    .unwrap_or_default();
                ps.sync_models(running);
            }
        }
    });
}

/// Send a native desktop toast notification across GNOME, KDE, and all Linux desktops.
pub fn notify(title: &str, body: &str) {
    // Try notify-send first (direct DBus notification to org.freedesktop.Notifications for GNOME/KDE).
    let res = std::process::Command::new("notify-send")
        .arg("-i")
        .arg("swai")
        .arg("-a")
        .arg("SWAI")
        .arg(title)
        .arg(body)
        .status();

    if res.is_ok() && res.unwrap().success() {
        return;
    }

    // Fallback to GIO Application notification if notify-send is unavailable.
    let notification = Notification::new("");
    notification.set_title(title);
    notification.set_body(Some(body));
    notification.set_icon(&gio::ThemedIcon::new("swai"));

    let app = gio::Application::default().and_then(|app| app.downcast::<adw::Application>().ok());

    if let Some(app) = app {
        app.send_notification(Some("swai-notification"), &notification);
    }
}
