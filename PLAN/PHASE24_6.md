# Phase 24f — GTK Debate Arena Desktop Window

> **Strict Architecture & File-Size Constraint (< 450 lines per file)**:
> Organize into `app/src/arena/` (< 350 lines per file).

---

## 1. Goal
Implement the GTK4 / Libadwaita **Debate Arena** window (`app/src/arena/`) allowing desktop users to view real-time turn-by-turn model deliberations and browse saved debate history.

---

## 2. Technical Scope

1. **`app/src/arena/history.rs`** (< 150 lines):
   - Save and load debate transcript JSON files to `~/.local/share/swai/debates/<id>.json`.
2. **`app/src/arena/view.rs`** (< 300 lines):
   - Turn-by-turn visual view: Generator Draft card (Cyan), Auditor Critiques card (Orange), Final Consensus card (Green).
3. **`app/src/arena/window.rs`** (< 300 lines):
   - `ArenaWindow` (Libadwaita window with history sidebar and live stream view).
4. **`app/src/arena/mod.rs`** (< 30 lines):
   - Re-exports and public API.

---

## 3. Verification Requirements
- `cargo check -p swai` passes with 0 errors and 0 warnings.
- `cargo test -p swai --bin swai` passes.
- All files strictly under 350 lines.

---

## 4. Progress Logging
- Commit locally: `git commit -m "feat(arena): P24f — GTK Debate Arena desktop window and history viewer"`
