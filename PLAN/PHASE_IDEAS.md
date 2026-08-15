# SWAI: Future Feature Ideas & Distribution Roadmap

This document tracks upcoming planned phases and future distribution/packaging ideas for post-v1.0 iterations.

---

## ⚡ TOP PRIORITY — Dynamic Model-Adaptive Context Compaction Budget

### Problem:
Different local models have drastically different context window capacities (e.g. 32k, 64k, 128k, 256k tokens). A single hardcoded compaction budget (e.g. 60k chars / ~16k tokens) is ideal for 64k models, but either overly restrictive for large-context models (128k/256k) or too loose for small 32k models.

### Proposed Architecture & Solution:
1. **Dynamic Context Scaling Ratio**:
   - Inspect the active model's configured `--ctx-size` (or `ctx_size` in `config.toml`).
   - Automatically compute the safe history compaction budget as a percentage of the total context window (e.g., default `25%` to `35%` of context window for conversation history, leaving `65%` to `75%` headroom for KV cache, system prompt, tools, and generation).
2. **Proportional Allocation Table**:
   - **32k context model**: History budget = ~10k tokens (~38k chars), leaving 22k tokens headroom.
   - **64k context model** (e.g., Ornith-35B): History budget = ~20k tokens (~75k chars), leaving 44k tokens headroom.
   - **128k context model** (e.g., Qwen-2.5-Coder / DeepSeek): History budget = ~45k tokens (~170k chars), leaving 83k tokens headroom.
   - **256k+ context model**: History budget = ~90k tokens (~340k chars), leaving 166k tokens headroom.
3. **Adaptive `tool_result` Truncation**:
   - Scale the per-tool-result truncation limit proportionally to the context budget (e.g., `min(ctx_tokens / 16, 16_000)` tokens per file read).
4. **Validation & Rollout**:
   - Test under real multi-step tasks across varying context windows in Phase 25/26.

---

## 🚀 Active Roadmap Summary (Phases 23 – 29)

- **Phase 23 — Multi-Model Concurrent Orchestration**: Run up to 4 models simultaneously with payload-based dynamic proxy routing.
- **Phase Between 23-24 — Context Checkpointing & Summarized Compaction**: Replace silent message eviction with session checkpoint summaries injected after system prompt.
- **Phase 24 — SWAI Council: Inter-Model Debate Broker & Synthetic Endpoint**: Multi-model peer review, critique, and synthesis pipeline with real-time GTK debate arena (sub-phases 24a–24g).
- **Phase Between 24-25 — Dynamic Model-Adaptive Context Compaction Budget**: Dynamically compute history compaction budgets and tool truncation limits based on the active model's configured context window (32k, 64k, 128k, 256k).
- **Phase 25 — Native Windows Port (WinUI 3 & System Tray)**: Bring native SWAI desktop experience to Windows.
- **Phase 26 — Native macOS Port (Cocoa & Menu Bar Item)**: Bring native SWAI desktop experience to macOS.
- **Phase 27 — Real-Time Hardware Telemetry (System RAM & Multi-Vendor VRAM Monitor)**: Display live System RAM and GPU VRAM memory usage across NVIDIA, AMD, Intel, and Apple Silicon.
- **Phase 28 — Cross-Platform Store & Package Manager Distribution**: Microsoft Store, `winget` repo, Homebrew Cask (`brew install`), and standalone `.dmg` packaging.
- **Phase 29 — Sidebar Preferences Redesign & UI Visual Polish**: KDE/GNOME-style sidebar tab navigation with preference search and UI visual polish.

---

## 📦 Future Distribution & Package Manager Ideas

### 1. Windows Distribution & Package Managers (Phase 25 Extension)
- **Microsoft Store (Win32 / MSIX)**: Publish free native WinUI 3 package directly to the Microsoft Store.
- **Windows Package Manager (`winget`)**: Submit manifest to official `microsoft/winget-pkgs` repository so users can install via single command:
  ```cmd
  winget install verdioso.swai
  ```
- **NSIS / WiX Installer**: Standalone setup executable (`SWAI-Setup-v1.x.x.exe`) with auto-updater support.

### 2. macOS Distribution & Package Managers (Phase 26 Extension)
- **Homebrew Cask Formula (`brew install`)**: Create official tap (`verdioso/tap`) so Mac developers can install SWAI instantly without Apple App Store fees:
  ```bash
  brew install verdioso/tap/swai
  ```
- **Standalone `.dmg` Disk Image**: Provide signed/notarized `.dmg` drag-and-drop installer for direct download from GitHub Releases.

---

## 🧹 Maintenance & Storage Management Ideas

### One-Click Log & Checkpoint Cleanup in Preferences
Add dedicated storage cleanup actions inside the Preferences dialog (System / Storage section) to let users easily manage and reclaim disk space:
- **Button 1: "Clear All Logs"**:
  - Deletes or truncates all model server and process logs in `~/.local/share/swai/logs/` (or `$XDG_DATA_HOME/swai/logs/`).
  - Shows a brief confirmation toast / banner upon completion.
- **Button 2: "Clear All Checkpoints"**:
  - Deletes all session checkpoint markdown files in `~/.local/share/swai/checkpoints/` (or `$XDG_DATA_HOME/swai/checkpoints/`).
  - Automatically resets any open Checkpoint tab in the Log Viewer to "No checkpoints recorded yet".
  - Shows a brief confirmation toast / banner upon completion.

---

*(Completed phases 1–22 and retired Phase 21 are documented in `PROGRESS.md`).*


