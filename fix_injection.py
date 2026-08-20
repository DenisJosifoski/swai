with open("core/src/proxy/anthropic.rs", "r") as f:
    content = f.read()

# 1. Update persist_checkpoint signature
old_sig = """fn persist_checkpoint(
    summary_lines: Vec<String>,
    initial_objective: Option<&str>,
    model_id: &str,
    state: &Arc<Mutex<ProxyState>>,
    target_port: u16,
) {"""

new_sig = """fn persist_checkpoint(
    json_val: &mut serde_json::Value,
    summary_lines: Vec<String>,
    initial_objective: Option<&str>,
    model_id: &str,
    state: &Arc<Mutex<ProxyState>>,
    target_port: u16,
) {"""
content = content.replace(old_sig, new_sig)

# 2. Update the caller
old_call = """                persist_checkpoint(
                    summary_lines,
                    initial_objective.as_deref(),
                    &model_id,
                    state,
                    target_port,
                );"""
new_call = """                persist_checkpoint(
                    json_val,
                    summary_lines,
                    initial_objective.as_deref(),
                    &model_id,
                    state,
                    target_port,
                );"""
content = content.replace(old_call, new_call)

# 3. Add the injection at the end
old_end = """    // NOTE: Checkpoint injection into the prompt is DISABLED.
    // The checkpoint log is still written to disk for diagnostics, but injecting
    // accumulated summaries back into the prompt creates a feedback loop on small
    // context windows (64k) where noise compounds into more compaction. This will
    // be re-enabled once dynamic model-adaptive context budgeting is fully tuned.
}"""
new_end = """    if let Some(checkpoint_text) = session.format_for_injection() {
        crate::compaction::inject_checkpoint_into_payload(json_val, &checkpoint_text);
    }
}"""
content = content.replace(old_end, new_end)

with open("core/src/proxy/anthropic.rs", "w") as f:
    f.write(content)
print("Done")
