# Phase 24b — Lightweight Hardware VRAM Probing for Council

> **Strict Architecture & File-Size Constraint (< 450 lines per file)**:
> Aim for < 150 lines in this module.

---

## 1. Goal
Implement a lightweight, zero-overhead VRAM memory probe (`core/src/council/vram.rs`) so Council's `mode = "auto"` can dynamically decide between concurrent and sequential execution based on available GPU VRAM.

---

## 2. Technical Scope

1. **`core/src/council/vram.rs`** (< 150 lines):
   - Implement `get_available_vram_bytes() -> Option<u64>`.
   - Try NVIDIA NVML via `nvml-wrapper` or direct query if available on Linux.
   - Fallback: parse Linux sysfs `/sys/class/drm/card*/device/mem_info_vram_used` or return `None` (graceful fallback).
   - Helper `recommend_mode(required_models_vram_bytes: u64) -> CouncilMode`:
     - If available VRAM >= required memory, return `CouncilMode::Concurrent`.
     - Otherwise, return `CouncilMode::Sequential`.

2. **Unit Tests**:
   - Unit tests in `core/src/council/tests.rs` verifying `recommend_mode` threshold logic and graceful fallback when no GPU is detected.

---

## 3. Verification Requirements
- `cargo check -p swai-core` passes with 0 warnings.
- `cargo test -p swai-core --lib council::tests` passes.
- File size strictly < 150 lines.

---

## 4. Progress Logging
- Commit locally: `git commit -m "feat(council): P24b — Hardware VRAM probe for Council auto mode"`
