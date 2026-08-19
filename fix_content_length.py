with open("core/src/proxy/router.rs", "r") as f:
    content = f.read()

old_is_hop = """pub fn is_hop_by_hop_header(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "connection"
            | "keep-alive"
            | "proxy-authenticate"
            | "proxy-authorization"
            | "te"
            | "trailer"
            | "transfer-encoding"
            | "upgrade","""

new_is_hop = """pub fn is_hop_by_hop_header(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "connection"
            | "content-length"
            | "keep-alive"
            | "proxy-authenticate"
            | "proxy-authorization"
            | "te"
            | "trailer"
            | "transfer-encoding"
            | "upgrade","""

if old_is_hop in content:
    content = content.replace(old_is_hop, new_is_hop)
    with open("core/src/proxy/router.rs", "w") as f:
        f.write(content)
    print("Success")
else:
    print("Failed to find block")
