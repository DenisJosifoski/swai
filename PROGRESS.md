# SWAI Progress Log

## Phase 19 — Instant VRAM Drop (`SIGINT` Process Manager Optimization) — Completed & Verified

- Changed primary stop signal from `SIGTERM` to `SIGINT` in `core/src/process_manager.rs` so `llama.cpp` unmaps CUDA/ROCm VRAM buffers instantly (~100ms).
- Fallback SIGKILL escalation preserved for orphan zombie prevention.
- `cargo check --workspace`: 0 errors, 0 warnings.
- `SWAI_NO_SINGLE_INSTANCE=1 cargo test --workspace`: all 134 tests pass (19 app unit + 111 core unit + 4 integration).
