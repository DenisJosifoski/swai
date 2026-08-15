# Phase 24g — Council Pipeline Preferences Tab

> **Strict Architecture & File-Size Constraint (< 450 lines per file)**:
> Modify `app/src/preferences/` submodules (< 350 lines per file).

---

## 1. Goal
Add a dedicated **Council Pipeline** tab inside the Preferences dialog (`app/src/preferences/`) allowing users to configure debate stages, roles, models, and prompt templates without hand-editing TOML.

---

## 2. Technical Scope

1. **`app/src/preferences/council_tab.rs`** (< 300 lines):
   - Pipeline stages list (`AdwPreferencesGroup`).
   - Add / Remove / Reorder turn buttons.
   - Role picker, model dropdown (populated from configured models), and prompt template entry per stage.
   - Mode selector (`Auto`, `Concurrent`, `Sequential`).
2. **`app/src/preferences/dialog.rs`**:
   - Add Council Pipeline tab to the Preferences window.
3. **Round-Trip Serialization**:
   - UI edits persist into `[[council.pipeline]]` in `config.toml`.

---

## 3. Verification Requirements
- `cargo check --workspace` passes with 0 errors and 0 warnings.
- Unit test verifying UI values <-> config TOML round-trip serialization.
- All files strictly under 400 lines.

---

## 4. Progress Logging
- Commit locally: `git commit -m "feat(preferences): P24g — Council Pipeline Preferences tab UI"`
- Append final Phase 24 summary to `PLAN/PROGRESS.md`.
