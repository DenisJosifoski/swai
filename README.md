# SWAI (SWitch AI) — Native Linux Model Switcher & Gateway

A native Linux desktop app for starting, stopping, and monitoring local
llama.cpp model servers, one at a time, from bash launch scripts. Written
entirely in Rust with a GTK4 + Libadwaita native shell UI — no webview, no Electron,
no Tauri.

## Features

- **Start / Stop / Switch** local llama.cpp model servers from a native
  GTK4 window with model cards.
- **System tray icon** with quick-switch menu, window visibility toggle,
  and clean quit — works natively on KDE Plasma.
- **Context display** — polls `/slots` to show token usage per model card,
  color-coded when approaching context limits.
- **Auto-restart** when context is full (configurable).
- **Reverse proxy** on a fixed local port so IDE/CLI clients never need
  reconfiguration when switching models.
- **Third-Party Inference Gateway** — connect Claude Desktop and other
  inference clients via `http://127.0.0.1:9080` with Anthropic Messages
  API support (`POST /v1/messages`, `GET /v1/models`).
- **Live log viewer** with auto-tail, clear, and export.

## Installation

### From source (requires Rust toolchain)

```bash
git clone <repo-url>
cd swai
cargo build --release
sudo cp target/release/swai /usr/local/bin/
```

Or use the provided install script:

```bash
./install.sh
```

## ⚙️ Configuration

`SWAI` reads its user configuration from `~/.config/swai/config.toml` (XDG Base Directory Specification):

> [!IMPORTANT]
> **Unique Ports**: Each configured model must have its own unique port (e.g. 8090, 8093, 8082). The port configured in `SWAI` must match the port specified in that model's launch script (e.g. `--port 8090`). All incoming queries from your tools are directed to the single proxy port (`9080`).

The config file defines models with their launch scripts and ports:

```toml
schema_version = 1

[[models]]
id = "my-model"
name = "My Model"
script_path = "/path/to/start-my-model.sh"
port = 9080
health_timeout_sec = 30

[global]
proxy_port = 9080
log_dir = "~/.local/share/swai/logs/"
auto_restart_on_context_full = true
```

Each model's launch script should include the `--slots` flag for context
display to work correctly. See the example script below.

### Example launch script

```bash
#!/bin/bash
# start-my-model.sh — launch a local llama.cpp server
exec llama-server \
    --model "$MODEL_PATH" \
    --port "$PORT" \
    --slots \                    # Required for context display
    --ctx-size 8192 \
    --batch-size 512
```

The script is expected to use `MODEL_PATH` and `PORT` environment variables.
The `--slots` flag is required for the context display feature.

### Third-Party Inference (Claude Desktop Gateway)

`SWAI` includes a reverse proxy that forwards all API requests to the
currently active model server on a fixed local port. This lets you connect
third-party inference clients like Claude Desktop without reconfiguring them
when you switch models.

#### Base URL

The proxy exposes a single Base URL for all API endpoints:

```
http://127.0.0.1:9080/v1
```

All incoming requests are forwarded transparently to whichever model is currently active in SWAI.

#### Verified Endpoints & API Formats

| API Spec | Endpoint | Method | Description |
|----------|----------|--------|-------------|
| **Anthropic Messages** | `/v1/messages` | POST | Claude Desktop & Claude Code CLI |
| **OpenAI Responses** | `/v1/responses` | POST | OpenAI Codex CLI & Codex Desktop |
| **OpenAI Chat** | `/v1/chat/completions` | POST | VS Code, Cursor, Continue.dev |
| **Ollama API** | `/api/generate`, `/api/chat` | POST | Ollama tools & CLI clients |
| **Model Discovery** | `/v1/models`, `/api/tags` | GET | Client model enumeration |

---

### 🚀 Client Setup Guides

#### 1. OpenAI Codex CLI Setup (`POST /v1/responses`)

To connect OpenAI Codex CLI to SWAI, configure `~/.codex/config.toml`:

```toml
model = "gpt-5.6-sol"
model_provider = "swai"
model_reasoning_effort = "low"

[model_providers.swai]
name = "SWAI Local AI"
base_url = "http://127.0.0.1:9080/v1"
wire_api = "responses"
api_key = "local"

[projects."/home/denisjosifoski"]
trust_level = "trusted"
```

Then simply launch:
```bash
codex
```
Codex will stream tokens live from your active SWAI model with zero disconnection errors!

#### 2. Claude Code CLI Setup (`claude-local`)

Add the following helper function to your `~/.bashrc`:

```bash
claude-local() {
  export ANTHROPIC_BASE_URL=http://localhost:9080
  export ANTHROPIC_AUTH_TOKEN=local
  export ANTHROPIC_API_KEY=""
  local live_model
  live_model=$(curl -s http://localhost:9080/v1/models | grep -o '"id":"[^"]*"' | head -1 | cut -d'"' -f4 | sed 's/^claude-//')
  export ANTHROPIC_MODEL="${live_model:-unknown}[1m]"
  export ANTHROPIC_SMALL_FAST_MODEL="$ANTHROPIC_MODEL"
  claude "${@}"
}
```

Then run `source ~/.bashrc`. When you launch `claude-local`, the banner header will dynamically display the active model name loaded in SWAI (e.g. `Qwen3.6-35B-A3B-UD-Q4_K_XL[1m]`).

#### 3. Claude Desktop Setup (Third-Party Inference → Gateway)

1. Enable **Developer Mode** in Claude Desktop's Help menu.
2. Open **Developer Menu** → **Configure Third-Party Inference** → **Gateway**.
3. Set **Gateway base URL** to `http://127.0.0.1:9080/` (or `http://127.0.0.1:9080`).
4. Set **Gateway API key** to `local` and **Gateway auth scheme** to `bearer`.
5. Under **Models**:
   - Enable **Model discovery** (toggle ON).
   - Under **Model list**, click **+ Add model**.
   - Set **Model ID** to `claude`.
   - Set **Display name** to `SWAI` (or any custom label).
   - Toggle **Offer 1M-context variant** and **Default to 1M context** ON.
6. Click **Apply Changes**. Claude Desktop displays **`SWAI 1M`** in its model picker!

#### 4. Ollama CLI & Tool Clients

Point any Ollama client or SDK to SWAI's port `9080`:

```bash
OLLAMA_HOST="http://127.0.0.1:9080" ollama run swai-model
```

#### Jinja Template Support

To serve Anthropic Messages API payloads, your model's launch script must include the `--jinja` flag (default-on in recent `llama-server` builds):

```bash
#!/bin/bash
exec llama-server \
    --model "$MODEL_PATH" \
    --port "$PORT" \
    --slots \
    --metrics \                  # Required for live tok/s telemetry
    --ctx-size 8192 \
    --batch-size 512 \
    --jinja \                    # Required for Anthropic Messages API
    --chat-template-file "$TEMPLATE_PATH"  # Optional: custom Jinja template
```

Tool-use support is verified per-model, not guaranteed by `--jinja` alone.

## System Tray Availability

`SWAI` uses a system tray icon for quick model toggling and
close-to-tray behavior. This requires a tray host (a StatusNotifierItem
implementation) to be running.

- **KDE Plasma**: works out of the box.
- **GNOME**: does not include a tray host by default. Install the
  **AppIndicator and KStatusNotifierItem Support** extension (actively
  maintained, available via extensions.gnome.org or your distro's
  repositories) to enable it.
- **Other desktops / minimal WMs (i3, sway, etc.)**: depends on whether you
  run a standalone indicator host (e.g. `nm-applet`-style tray daemons).

If no tray host is detected at startup, `SWAI` automatically disables
"Minimize to Tray" and only offers "Quit" on window close, so you're never
left with a hidden window and no way to bring it back.

## Architecture

```
swai/
├── Cargo.toml              # workspace root
├── core/                   # library crate — config, process lifecycle,
│                           #   health monitoring, reverse proxy, single instance
├── app/                    # binary crate — GTK4 UI, system tray, menus
├── install.sh
└── README.md
```

The `core` crate has zero GTK dependency and can be tested independently
with `cargo test`.

## License

MIT
