# Phase Between 24–25 — Dynamic Model-Adaptive Context Compaction Budget

> **Strict Architecture & File-Size Constraint (< 450 lines per file)**:
> Keep all changes modularized under `core/src/compaction/` and `core/src/proxy/`.
> No file may exceed 450 lines (aim for < 300 lines).

---

## 1. Goal
Replace static compaction constants with **dynamic, model-adaptive context budgeting**. SWAI will automatically inspect the active model's configured context window capacity (`ctx_size` / `--ctx-size`, e.g. 32k, 64k, 128k, 256k) and dynamically scale conversation history budgets and tool result truncation limits proportionally.

---

## 2. Technical Architecture & Formulas

### A. Dynamic Allocation Ratio (`core/src/compaction/budget.rs` or `types.rs`)
1. **Target History Ratio (`35%` default)**:
   - For a given model context window $N_{\text{ctx}}$ tokens:
     $$\text{Budget}_{\text{tokens}} = \lfloor N_{\text{ctx}} \times 0.35 \rfloor$$
     $$\text{Budget}_{\text{chars}} = \text{Budget}_{\text{tokens}} \times 4$$
   - **Headroom (`65%`)**: Guaranteed reserved context space for system prompt, tool definitions, tool results, and output generation.

2. **Context Scaling Reference Table**:
   | Model Context Window ($N_{\text{ctx}}$) | History Token Budget (35%) | History Character Budget | Headroom for Tools / Output |
   | :--- | :---: | :---: | :---: |
   | **32,768 (32k)** | ~11,400 tokens | ~45,000 chars | ~21,300 tokens |
   | **65,536 (64k)** (e.g. Ornith-35B) | ~22,900 tokens | ~91,000 chars | ~42,600 tokens |
   | **131,072 (128k)** (e.g. Qwen-2.5-Coder) | ~45,800 tokens | ~183,000 chars | ~85,200 tokens |
   | **262,144 (256k)** (e.g. DeepSeek / Llama-3.1) | ~91,700 tokens | ~366,000 chars | ~170,400 tokens |

3. **Adaptive Tool Result Truncation**:
   - Instead of a hardcoded 8,000 character limit, compute per-tool-result truncation dynamically:
     $$\text{ToolTruncation}_{\text{chars}} = \min\left(\lfloor N_{\text{ctx}} \times 0.10 \times 4 \rfloor, 64\,000\right)$$
   - Minimum clamp: 8,000 chars. Maximum clamp: 64,000 chars.

---

## 3. Implementation Steps

1. **Context Extraction & Model Lookup**:
   - In `core/src/config/model.rs` / `core/src/process_manager/`: ensure `ctx_size` (inferred from script args `--ctx-size <N>` or config field) is stored in model metadata and queryable by `target_port` or `model_id`.
   - Fallback: if context size cannot be determined, default to 65,536 (64k).

2. **Integrate into `core/src/compaction/eviction.rs`**:
   - Pass the active model's `ctx_size` into `compact_messages_anthropic(messages, config, ctx_size)`.
   - Calculate `max_budget_chars` and tool truncation limits dynamically using the formula above.

3. **Integrate into `core/src/proxy/anthropic.rs`**:
   - Look up the active model's `ctx_size` from `state` and pass it into the compaction pipeline.
   - Adjust the pre-compaction request size check threshold dynamically:
     $$\text{TriggerThreshold}_{\text{chars}} = \text{Budget}_{\text{chars}} \times 0.90$$

---

## 4. Verification Requirements

1. **Unit Tests (`core/src/compaction/tests_basic.rs` & `tests_anthropic.rs`)**:
   - Test dynamic budget calculation across 32k, 64k, 128k, and 256k context models.
   - Verify that a 128k model retains ~180k chars without evicting, while a 32k model evicts early to maintain headroom.
   - Verify tool result truncation scales proportionally with context size.
2. **Compiler & Cleanliness**:
   - `cargo check --workspace` — 0 errors, 0 warnings.
   - `cargo test --workspace` passes 100%.
   - All files strictly under 450 lines.

---

## 5. Progress Logging

- Commit locally: `git commit -m "feat(compaction): Phase Between 24-25 — Dynamic Model-Adaptive Context Compaction Budget"`
- Append summary entry to `PLAN/PROGRESS.md`.
