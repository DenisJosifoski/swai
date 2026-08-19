#!/bin/bash
# Remove the incorrect block from response handling
sed -i '/\/\/ Process anthropic payloads if needed./,/^    }$/d' core/src/proxy/router.rs

# Insert the block to process the request body right after reading it
awk '
/let request_body_len = request_body.len();/ {
    print
    print "    if path_and_query.contains(\"/v1/messages\") && req.method().as_str() == \"POST\" {"
    print "        if let Ok(mut json_val) = serde_json::from_slice::<serde_json::Value>(&request_body) {"
    print "            process_anthropic_payload(&mut json_val, request_body_len, &state, target_port_val);"
    print "            if let Ok(serialized) = serde_json::to_vec(&json_val) {"
    print "                request_body = serialized;"
    print "            }"
    print "        }"
    print "    }"
    next
}
{ print }
' target_port_val="target_port.unwrap_or(0)" core/src/proxy/router.rs > tmp_router.rs

# We need to make sure target_port is resolved BEFORE we process the payload!
# Actually, target_port is resolved further down. Let's do it safely.
