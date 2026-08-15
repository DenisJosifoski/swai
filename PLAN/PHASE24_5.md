# Phase 24e — Reverse Proxy Route Integration

> **Strict Architecture & File-Size Constraint (< 450 lines per file)**:
> Modify `core/src/proxy/router.rs` (< 420 lines).

---

## 1. Goal
Hook SWAI Council into the reverse proxy router so requests for synthetic model `"council"` or containing `X-SWAI-Pipeline` headers route to the Council broker.

---

## 2. Technical Scope

1. **`core/src/proxy/router.rs`**:
   - Intercept requests where `model == "council"` or `model.starts_with("council:")`.
   - Parse `X-SWAI-Pipeline` HTTP header if provided for dynamic pipeline overrides.
   - Dispatch request to `CouncilEngine` and return streamed response.

2. **Integration Tests**:
   - Unit tests in `core/src/proxy/tests_protocol.rs` verifying council route interception and header extraction.

---

## 3. Verification Requirements
- `cargo test -p swai-core --lib` passes 100%.
- `router.rs` stays strictly under 430 lines.

---

## 4. Progress Logging
- Commit locally: `git commit -m "feat(proxy): P24e — Proxy route integration for Council synthetic endpoint"`
