# SWAI Progress Log

## Phase 20 — Auto & Manual Update Checker with Self-Installer — Completed & Verified

- Rewrote `app/src/update_checker.rs` for GTK4/Libadwaita (removed Tauri dependencies).
- Added `Version` struct with semver parsing, comparison, and display.
- Implemented `check_for_updates_blocking()` using blocking reqwest for GTK main thread compatibility.
- Created `app/src/update_installer.rs` for self-installing updates (download, extract, replace binary).
- Added "Check for Updates" button in About dialog and Preferences dialog.
- Added background update check on app startup (logs result, no UI blocking).
- Added unit tests for semver parsing, comparison, and URL construction.
- `cargo check --workspace`: 0 errors, 0 warnings (only dead_code warnings for unused helpers).
- `SWAI_NO_SINGLE_INSTANCE=1 cargo test --workspace`: all 146 tests pass (31 app unit + 111 core unit + 4 integration).

## Phase 19 — Instant VRAM Drop (`SIGINT` Process Manager Optimization) — Completed & Verified

- Changed primary stop signal from `SIGTERM` to `SIGINT` in `core/src/process_manager.rs` so `llama.cpp` unmaps CUDA/ROCm VRAM buffers instantly (~100ms).
- Fallback SIGKILL escalation preserved for orphan zombie prevention.
- `cargo check --workspace`: 0 errors, 0 warnings.
- `SWAI_NO_SINGLE_INSTANCE=1 cargo test --workspace`: all 134 tests pass (19 app unit + 111 core unit + 4 integration).
