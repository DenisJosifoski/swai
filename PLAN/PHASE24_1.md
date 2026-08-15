# Phase 24a — Council Data Types & Core Module Layout

> **Strict Architecture & File-Size Constraint (< 450 lines per file)**:
> All code in this sub-phase must remain strictly modularized.
> Aim for < 250 lines per file.

---

## 1. Goal
Define all data models, enums, pipeline configuration structures, and serialization logic for SWAI Council inside `core/src/council/`.

---

## 2. Technical Scope

1. **`core/src/council/types.rs`** (< 200 lines):
   - `CouncilMode`: `Auto`, `Concurrent`, `Sequential` (with `serde` support).
   - `CouncilRole`: `Generator`, `Auditor`, `Synthesizer`, `Custom(String)`.
   - `PipelineStage`: Model ID, role, prompt template, temperature/sampling options.
   - `CouncilPipelineConfig`: List of stages, execution mode, fallback options.
   - `TurnResult`: Status, role, model, output text, duration, error details.
   - `DebateTranscript`: Full record of debate turns for persistence.

2. **`core/src/council/mod.rs`** (< 50 lines):
   - Module declarations: `pub mod types;`, `pub mod vram;`, `pub mod pipeline;`, `pub mod streaming;`, `#[cfg(test)] mod tests;`
   - Re-exports of public types.

3. **`core/src/council/tests.rs`** (< 200 lines):
   - Unit tests verifying JSON & TOML round-trip serialization of `CouncilPipelineConfig` and `DebateTranscript`.

4. **Register in `core/src/lib.rs`**:
   - `pub mod council;`

---

## 3. Verification Requirements
- `cargo check -p swai-core` passes with 0 errors and 0 warnings.
- `cargo test -p swai-core --lib council::tests` passes 100%.
- All files strictly under 250 lines.

---

## 4. Progress Logging
- Commit locally: `git commit -m "feat(council): P24a — Council data types and module structure"`
