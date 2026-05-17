# SessionGraph QA Code Review — Full Report

**Date:** 2025-07-11
**Scope:** Full codebase audit against `SESSIONGRAPH_SPEC.md` v1.0
**Verdict:** **FAILS QA.** 3 critical bugs, 5 high-priority, 6 medium, 7 low. Cannot ship.

---

## 🔴 CRITICAL (blocks release)

### C1. Output tokens never written to `requests` table

**File:** `proxy/intercept.rs` lines ~164-186, ~255-278

The `PipelineLog` is built with `tokens_out: 0`. A background task with a **3-second sleep** tries to update the session table afterward, but:

1. `log_request()` is spawned *immediately* — it writes `tokens_out: 0` to the `requests` table before the delayed task runs.
2. The delayed task only calls `increment_session()` — it **never updates the `requests.tokens_out` column**.

**Result:** Every request row in the database has `tokens_out = 0` forever. Cost calculations that include output tokens are wrong. The `stats` endpoint computes `tokens_saved` from `tokens_in_raw - tokens_in_sent`, which ignores output tokens.

**Fix:** Don't log the request until output tokens are known. Either block on the stream completing, or use a two-phase log (insert with defaults, then UPDATE).

---

### C2. Token double-counting in sessions table

**File:** `proxy/intercept.rs` lines ~318-340 (`manage_session`)

`manage_session` increments `tokens_in_raw` and `tokens_in_sent` in the in-memory `ActiveSession`, and also calls `insert_session()` for new sessions. Then `log_request()` calls `increment_session()` **again** with the same deltas (line ~391).

**Result:** Session token counts in the DB are **doubled** — every request's tokens are added twice.

**Fix:** `manage_session` should only manage session lifecycle (create/end), not accumulate DB counters. Let `log_request` be the sole writer of token counts to the DB. The in-memory counters should be reconstructed from DB on startup if needed.

---

### C3. Compression subprocess path is a fantasy

**File:** `proxy/compress.rs` lines ~126-148

The code looks for `~/.sessiongraph/venv/Scripts/headroom-compress.py` (Windows) or `~/.sessiongraph/venv/bin/headroom-compress.py` (Unix). But `venv.rs` runs `pip install headroom-ai`, which installs a Python **package**, not a standalone `.py` script in `bin/` or `Scripts/`.

Per spec §5.4, the call should be:
```
~/.sessiongraph/venv/bin/python -m headroom.compress --input-json ... --mode token --output-json
```

**Result:** Compression **always fails silently** (script never exists). Every request falls through to uncompressed forwarding. The entire compression feature is dead code.

**Fix:** Rewrite `compress()` to invoke `python -m headroom.compress` with the correct arguments, piping JSON via stdin.

---

## 🟠 HIGH (must fix before ship)

### H1. Session identity missing API key hash

**File:** `proxy/intercept.rs` line ~126, ~224

Spec §2.3: "Session identity is derived from: Provider, Project hash, Tool identity." But spec §5.3 step 2 says "Look up active session for **(api_key_hash, project_hash)**."

The implementation uses only `(project_hash, provider)`. Two developers on the same machine with different API keys but the same project would merge into one session. The `api_key` is passed to `manage_session` but **ignored** in the session lookup.

### H2. `log_request` races with the delayed output-token task

**File:** `proxy/intercept.rs` lines ~164-195

`log_request()` writes to `requests` and calls `increment_session()` immediately. The delayed task (3s sleep) calls `increment_session()` again for output tokens. These race: the delayed task may run before or after `log_request`. If after, the session's `tokens_out` gets incremented but the request row stays at 0. If before, the session counter might also be off.

### H3. `graph_max_tokens` setting never enforced

**File:** `graph/injector.rs`

The spec §2.4: "Token budget for session graph injection: maximum 500 tokens." The setting is stored in the DB and surfaced in the Settings UI, but the injector prepends the **full** graph JSON without truncation. A large graph bloats the system prompt beyond budget.

### H4. Sessions never ended on app shutdown

**File:** `lib.rs` lines ~80-93

`ProxyShutdown::drop` sends a shutdown signal to the Axum server, but active sessions remain in 'active' status in the DB. No code iterates `active_sessions` to call `end_session()` and trigger extraction.

### H5. `log_error()` function exists but is never called anywhere

**File:** `db/mod.rs` lines ~56-63

The `log_error()` function writes to `~/.sessiongraph/logs/error.log` per spec §5.5. It's defined but **zero callsites** exist in the entire codebase. Internal errors go to `tracing::error!` but never to the error log file required by spec.

---

## 🟡 MEDIUM (should fix before ship)

### M1. `GET /sessions` REST endpoint missing from Axum server

**Spec §5.2** lists `GET /sessions` as a proxy endpoint returning paginated session list. Only the Tauri IPC command exists. The REST endpoint is missing from `server.rs`.

### M2. Graph extraction uses potentially empty API key

**File:** `proxy/intercept.rs` ~line 140

`extract_api_key()` can return `None` — the code falls back to `unwrap_or_default()` giving an empty string. An empty API key is passed to `session.api_key`, stored in `ActiveSession`, and later used by `extract_anthropic`/`extract_openai`. The extraction API call will fail with 401.

### M3. 7-day token usage chart missing from dashboard

**Spec §6.3** shows a recharts bar chart: "TOKEN USAGE — LAST 7 DAYS [recharts bar chart: raw vs compressed per day]". The Dashboard component has no chart at all. The `token_usage_daily` table is populated correctly, but no query exists to fetch the 7-day data.

### M4. "Delete all data" in Settings is a no-op

**File:** `components/Settings.tsx` line ~67

The delete button shows a confirm dialog, then just `console.log("Data reset requested")`. No data is actually deleted. No Tauri command exists to wipe the DB.

### M5. Settings shows hardcoded version "0.1.0"

**File:** `components/Settings.tsx` line ~140

```tsx
SessionGraph v{settings.proxy_port ? "0.1.0" : "loading…"}
```

Uses `proxy_port` as a truthiness check and hardcodes the version. Should use the actual app version from Tauri's `getVersion()` or a backend command.

### M6. `get_session` Tauri command fetches 1000 rows and filters locally

**File:** `commands/sessions.rs` lines ~72-79

Instead of a direct `WHERE id = ?` query, it calls `list_sessions_paginated(1, 1000)` and does a linear scan. Wasteful and breaks when there are >1000 sessions.

---

## 🔵 LOW (fix when convenient)

### L1. `spawn_token_counter()` in forward.rs is dead code
Defined at line ~28. Never called anywhere. Remove or wire up.

### L2. `once_cell` dependency in Cargo.toml is unused
`std::sync::OnceLock` is used instead (stable since Rust 1.70). The `once_cell` crate can be removed.

### L3. `SessionGraph` struct in schema.rs is unused by extractor/injector
The extractor builds the JSON schema inline with `serde_json::json!`. The injector passes raw JSON strings. The well-typed `SessionGraph` struct is never used outside of `parse_and_validate_graph`.

### L4. `stats_handler` only returns first active session
Uses `sessions.first()` — if multiple projects are active simultaneously, only one is shown. Should find the most relevant session or return all.

### L5. Migration `001_init.sql` has a typo in comment: `YYYYY-MM-DD`
Line in `token_usage_daily` date comment says `YYYY-MM-DD` (extra Y). Harmless but sloppy.

### L6. `restart_proxy` Tauri command says "restart the app to complete restart"
The command sends a signal that shuts down the Axum server, but nothing restarts it. The spec says this should be a functional restart. Current implementation requires a full app restart.

### L7. No `GET /sessions/:id/graph` endpoint — uses `/:project_hash/graph` instead
Spec says path param is `:id` (session id), implementation uses `:project_hash`. Different semantics — project_hash gives the latest graph, session id would give that specific session's graph.

---

## 📋 Spec Alignment Checklist

| Spec Section | Feature | Status |
|---|---|---|
| §4 | Database schema (5 tables) | ✅ Matches |
| §5.2 | POST /v1/messages | ✅ |
| §5.2 | POST /v1/chat/completions | ✅ |
| §5.2 | GET /health | ✅ |
| §5.2 | GET /stats | ✅ |
| §5.2 | GET /sessions | ❌ Missing |
| §5.2 | GET /sessions/:id/graph | ⚠️ Mismatched param |
| §5.3 | Request pipeline (6 steps) | ✅ |
| §5.4 | Headroom compression via venv | ❌ Broken (C3) |
| §5.5 | Error logging to file | ⚠️ Unused fn (H5) |
| §5.5 | Proxy never breaks workflow | ✅ Good design |
| §6.3 | Dashboard with stat cards | ✅ |
| §6.3 | 7-day token chart | ❌ Missing (M3) |
| §6.4 | Session list items | ✅ |
| §6.5 | Session graph viewer | ✅ |
| §6.6 | 4-step onboarding | ✅ |
| §6.7 | Settings panel | ⚠️ Data mgmt incomplete |
| §7 | All Tauri IPC commands | ✅ |
| §9.3 | Auto-updater config | ✅ |

---

## 🧪 What Actually Works

- Proxy server starts, binds port 4200, routes Anthropic and OpenAI requests
- Streaming (SSE) pass-through forwarding works
- Session detection and 30-minute timeout logic is correct
- Database initialization and migration runs cleanly
- Graph extraction fires at session end (if API key available)
- Graph injection prepends context to system prompt
- Dashboard UI renders with live polling
- Session list populates from DB
- Session detail renders graph as readable cards
- Settings UI toggles write to DB
- Onboarding wizard flow is complete
- Output token byte counting (the counter itself works, just isn't persisted correctly)
- Provider auto-detection (Anthropic, OpenAI, OpenRouter)

---

## 🎯 Minimum Fixes Required To Ship

1. **C1** — Fix output tokens persistence (two-phase log or block on stream completion)
2. **C2** — Remove double-counting in session token accumulation
3. **C3** — Rewrite compression to call `python -m headroom.compress` per spec
4. **H1** — Include API key hash in session identity
5. **H2** — Synchronize log_request with output token task
6. **H3** — Enforce graph_max_tokens budget in injector
7. **H4** — End active sessions on app shutdown
8. **H5** — Wire up `log_error()` calls at actual error sites

All of these are self-contained, 1-file fixes except C1/C2 which require restructuring the pipeline log flow.
