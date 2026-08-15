# Phase 24c — Multi-Turn Debate Pipeline & Fallback Execution Engine

> **Strict Architecture & File-Size Constraint (< 450 lines per file)**:
> Keep `pipeline.rs` strictly under 350 lines.

---

## 1. Goal
Implement the core Council execution engine (`core/src/council/pipeline.rs`) coordinating the multi-turn Generator $\rightarrow$ Auditor $\rightarrow$ Synthesizer workflow with failure matrix resilience.

---

## 2. Technical Scope

1. **`core/src/council/pipeline.rs`** (< 350 lines):
   - `CouncilEngine` struct referencing `ProcessManager` and `ProxyState`.
   - Stage 1 — **Generator**: Prompt the primary model to generate candidate draft response.
   - Stage 2 — **Auditor(s)**: Forward prompt + candidate draft to auditor model(s) for critique, fact-checking, or security review.
   - Stage 3 — **Synthesizer**: Send original prompt + draft + audit critiques to synthesizer model to produce final response.
   - **Failure Matrix**: If an auditor or synthesizer times out or fails, gracefully return the best available draft with a warning note.
   - Support both `Concurrent` (parallel queries to running models) and `Sequential` (fast process swap < 500ms) execution.

2. **Unit Tests**:
   - Mock pipeline execution tests in `core/src/council/tests.rs` with simulated stage completions and failure fallback scenarios.

---

## 3. Verification Requirements
- `cargo test -p swai-core --lib council::tests` passes cleanly.
- `core/src/council/pipeline.rs` strictly < 350 lines.

---

## 4. Progress Logging
- Commit locally: `git commit -m "feat(council): P24c — Council multi-turn debate execution engine"`
