import re

with open("core/src/proxy/router.rs", "r") as f:
    content = f.read()

# 1. Remove the old block
old_block = """    // Process anthropic payloads if needed.
    let mut processed_bytes = response_bytes.to_vec();
    if path_and_query.contains("/v1/messages") && method_str == "POST" {
        if let Ok(mut json_val) = serde_json::from_slice::<serde_json::Value>(&processed_bytes) {
            process_anthropic_payload(&mut json_val, request_body_len, &state, target_port);
            if let Ok(serialized) = serde_json::to_vec(&json_val) {
                processed_bytes = serialized;
            }
        }
    }"""
content = content.replace(old_block, "    let mut processed_bytes = response_bytes.to_vec();")

# 2. Insert the new block
new_block = """    let method_str = req.method().as_str();

    // Process anthropic payloads on the request body BEFORE forwarding
    if path_and_query.contains("/v1/messages") && method_str == "POST" {
        if let Ok(mut json_val) = serde_json::from_slice::<serde_json::Value>(&request_body) {
            process_anthropic_payload(&mut json_val, request_body_len, &state, target_port);
            if let Ok(serialized) = serde_json::to_vec(&json_val) {
                request_body = serialized;
            }
        }
    }

    let mut builder = client.request("""

content = content.replace("""    let method_str = req.method().as_str();
    let mut builder = client.request(""", new_block)

with open("core/src/proxy/router.rs", "w") as f:
    f.write(content)
